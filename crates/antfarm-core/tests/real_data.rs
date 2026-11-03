//! Sanity check against a real transcript. Ignored by default; run with
//!
//! ```text
//! ANTFARM_PARSE_FILE=~/.claude/projects/<slug>/<session>.jsonl \
//!   cargo test -p antfarm-core --release --test real_data -- --ignored --nocapture
//! ```
//!
//! Feeds the last 16 MiB through the tailer, parser, and engine, prints the root agent's
//! usage, cost, context, and model, runs the backward cost-state scan, and compares the
//! price table against the `costUSD` Claude Code itself recorded per model.

use std::path::PathBuf;

use antfarm_core::claude::cost::cost_anchor;
use antfarm_core::claude::parse::{Parsed, parse_batch, parse_line};
use antfarm_core::claude::price;
use antfarm_core::input::{AgentRef, Input};
use antfarm_core::tail::{TailConfig, Tailer, find_last_line_containing};
use antfarm_core::{Engine, ParseStats, SessionId, Usage};

#[test]
#[ignore]
fn real_file_through_engine() {
    let Some(path) = std::env::var_os("ANTFARM_PARSE_FILE").map(PathBuf::from) else {
        eprintln!("ANTFARM_PARSE_FILE not set; skipping");
        return;
    };
    let session = SessionId::from("real");
    let mut engine = Engine::new();
    let t = std::time::Instant::now();
    let mut tailer = Tailer::attach(&path, TailConfig::default()).unwrap();
    engine.apply(Input::Attached {
        session: session.clone(),
        agent: AgentRef::Root,
        path: path.clone(),
        truncated_before: tailer.truncated_before(),
    });
    let anchor = cost_anchor(&path, 128 << 20).unwrap();
    println!("cost_anchor (128 MB scan): {anchor:?}");
    if let Some((usd, offset, at)) = anchor {
        engine.apply(Input::CostAnchor {
            session: session.clone(),
            usd,
            offset,
            at,
        });
    }
    let poll = tailer.poll().unwrap();
    let mut stats = ParseStats::default();
    let (lines, errors) = parse_batch(poll.lines, &mut stats);
    let events = engine.apply(Input::Lines {
        session: session.clone(),
        agent: AgentRef::Root,
        lines,
        errors,
        end_offset: poll.end_offset,
        mtime: None,
    });
    let s = &engine.machine().sessions[&session];
    let last = s.last_write.unwrap();
    engine.apply(Input::Tick {
        now: last + chrono::Duration::seconds(1),
    });
    let s = &engine.machine().sessions[&session];
    let root = s.root_agent();
    println!("elapsed {:?}, {} events", t.elapsed(), events.len());
    println!(
        "parse: lines {} bytes {} parsed {} errors {} truncated_before {:?}",
        s.parse.lines, s.parse.bytes, s.parse.parsed, s.parse.errors, s.truncated_before
    );
    println!("status {:?} since {:?}", s.status, s.status_since);
    println!("root model {:?} effort {:?}", root.model, root.effort);
    println!("root usage {:#?}", root.usage);
    println!("root cost_usd {:?}", root.cost_usd);
    println!("root context {:?}", root.context);
    println!("session cost {:#?}", s.cost);
    println!("session lines +{:?} -{:?}", s.lines_added, s.lines_removed);
    println!(
        "open calls {:?}",
        root.open.values().map(|c| &c.name).collect::<Vec<_>>()
    );

    // Implied-rate cross-check against Claude Code's own per-model cost.
    let Some(raw) = find_last_line_containing(&path, br#""type":"cost-state""#, 1 << 30).unwrap()
    else {
        println!("no cost-state line in the first GiB from the end");
        return;
    };
    let Ok(Parsed::CostState(c)) = parse_line(&raw.bytes, raw.offset) else {
        panic!("cost-state line does not parse");
    };
    println!(
        "cost-state total {:?} at offset {}",
        c.total_cost_usd, raw.offset
    );
    for (model, mu) in c.model_usage.unwrap_or_default() {
        let u = Usage {
            input: mu.input_tokens,
            output: mu.output_tokens,
            cache_read: mu.cache_read_input_tokens,
            cache_creation: mu.cache_creation_input_tokens,
            cache_creation_1h: mu.cache_creation_ephemeral_1h_input_tokens.unwrap_or(0),
            thinking: mu.thinking_tokens.unwrap_or(0),
            messages: 0,
        };
        // Base rates, not `price_usage`: this is a session aggregate, and Haiku 5.5's
        // over-100K tier is decided per request. Web searches bill at $10 per 1,000.
        let ours = price::lookup(&model)
            .map(|m| m.rates.price(&u) + mu.web_search_requests.unwrap_or(0) as f64 * 0.01);
        let theirs = mu.cost_usd;
        let ratio = match (ours, theirs) {
            (Some(o), Some(t)) if t > 0.0 => Some(o / t),
            _ => None,
        };
        println!(
            "{model}: table {ours:?} vs costUSD {theirs:?} ratio {ratio:?} (web searches {:?}, 1h writes {:?})",
            mu.web_search_requests, mu.cache_creation_ephemeral_1h_input_tokens
        );
    }
}
