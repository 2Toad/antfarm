//! `antfarm doctor`: what antfarm found, what it could not read, and whether its prices
//! agree with the costs Claude Code itself records.

use std::collections::BTreeMap;

use antfarm_core::Usage;
use antfarm_core::claude::line::CostStateLine;
use antfarm_core::claude::parse::{Parsed, parse_line};
use antfarm_core::claude::price;
use antfarm_core::tail::find_last_line_containing;

use super::{home_relative, settle, short_status, start_live};

#[derive(Debug, clap::Args)]
pub struct Args {}

pub async fn run(_: Args) -> anyhow::Result<()> {
    let live = start_live();
    settle(&live).await;
    let snap = live.handle.snapshot();
    live.handle.stop();
    let st = live.stats.lock().unwrap().clone();
    let paths = antfarm_core::Paths::from_env();

    println!("antfarm {}", env!("CARGO_PKG_VERSION"));
    println!("claude home     {}", home_relative(&paths.claude_home));
    println!("file watcher    {}", st.backend);
    println!("passes          {}", st.passes);
    println!();

    println!("Registry ({} entries)", st.registry.len());
    for r in &st.registry {
        match &r.entry {
            Ok(e) => println!(
                "  pid {:<7} {}  {:<8} {:<20} {}  {}",
                r.pid,
                if r.alive { "alive" } else { "dead " },
                e.status.as_deref().unwrap_or("-"),
                e.waiting_for
                    .as_deref()
                    .map(|w| format!("({w})"))
                    .unwrap_or_default(),
                e.version.as_deref().unwrap_or("-"),
                e.session_id
            ),
            Err(err) => println!("  pid {:<7} unreadable: {err}", r.pid),
        }
    }
    println!();

    println!("Sessions");
    for s in snap.machine.sessions.values() {
        let path = st
            .transcripts
            .iter()
            .find(|(id, _)| id == &s.id)
            .and_then(|(_, p)| p.clone());
        let dirs = st
            .session_dirs
            .iter()
            .find(|(id, _)| id == &s.id)
            .map(|(_, d)| d.clone())
            .unwrap_or_default();
        println!("  {} {}", s.id, s.name.as_deref().unwrap_or(""));
        println!("    state         {}", short_status(s.status));
        match (&path, s.status) {
            (Some(p), _) => println!("    transcript    {}", home_relative(p)),
            (None, antfarm_core::SessionStatus::Exited) => {
                println!("    transcript    not read (process gone)")
            }
            (None, _) => println!("    transcript    NOT FOUND"),
        }
        if let Some(t) = s.truncated_before {
            println!("    history       read from byte {t} on");
        }
        println!(
            "    sub-agent dirs {}  workflow dirs {}",
            dirs.subagents.len(),
            dirs.workflows.len()
        );
        println!(
            "    agents        {} ({} finished)  workflows {}",
            s.agents.len(),
            s.agents.values().filter(|a| a.finished.is_some()).count(),
            s.workflows.len()
        );
        let p = &s.parse;
        println!(
            "    lines         {} read, {} decoded, {} unreadable, {} orphan results ({} after truncation)",
            p.lines, p.parsed, p.errors, p.orphan_results, p.orphan_after_truncation
        );
        if let Some(e) = &p.first_error {
            println!("    first error   {e}");
        }
        let unknown: Vec<_> = s
            .agents
            .values()
            .filter(|a| a.usage.messages > 0)
            .filter_map(|a| a.model.as_deref())
            .filter(|m| price::lookup(m).is_none())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !unknown.is_empty() {
            println!("    unpriced models {}", unknown.join(", "));
        }
        if let Some(p) = path {
            rate_check(&p);
        }
    }
    println!();

    println!("Totals");
    println!("  files tailed    {}", st.files_tailed);
    println!("  bytes read      {}", st.bytes_read);
    println!(
        "  lines           {} read, {} decoded, {} unreadable, {} oversized",
        st.parse.lines,
        st.parse.parsed,
        st.parse.errors,
        st.parse.oversized + st.oversized
    );
    let versions: Vec<String> = st
        .parse
        .versions
        .iter()
        .map(|(v, n)| format!("{v} ({n})"))
        .collect();
    println!(
        "  claude versions {}",
        if versions.is_empty() {
            "-".into()
        } else {
            versions.join(", ")
        }
    );
    let mut ignored: Vec<_> = st.parse.ignored_types.iter().collect();
    ignored.sort_by(|a, b| b.1.cmp(a.1));
    let ignored: Vec<String> = ignored
        .iter()
        .take(8)
        .map(|(t, n)| format!("{t} {n}"))
        .collect();
    println!("  skipped types   {}", ignored.join(", "));
    for e in &st.errors {
        println!("  error           {e}");
    }
    let problems = st.parse.errors
        + snap
            .machine
            .sessions
            .values()
            .map(|s| s.parse.errors)
            .sum::<u64>();
    println!();
    println!(
        "{}",
        if problems == 0 && st.errors.is_empty() {
            "No problems found."
        } else {
            "Problems found; see above."
        }
    );
    Ok(())
}

/// Prices each model's lifetime usage from the session's last cost-state with antfarm's
/// table and compares it with the cost Claude Code recorded.
fn rate_check(path: &std::path::Path) {
    let Ok(Some(line)) = find_last_line_containing(path, br#""type":"cost-state""#, 128 << 20)
    else {
        return;
    };
    let Ok(Parsed::CostState(c)) = parse_line(&line.bytes, line.offset) else {
        return;
    };
    let CostStateLine {
        model_usage: Some(models),
        ..
    } = *c
    else {
        return;
    };
    let mut rows = BTreeMap::new();
    for (model, m) in models {
        let one_h = m.cache_creation_ephemeral_1h_input_tokens.unwrap_or(0);
        let u = Usage {
            input: m.input_tokens,
            output: m.output_tokens,
            cache_read: m.cache_read_input_tokens,
            cache_creation: m.cache_creation_input_tokens,
            cache_creation_1h: one_h,
            ..Usage::default()
        };
        let ours = price::lookup(&model)
            .map(|p| p.rates.price(&u) + m.web_search_requests.unwrap_or(0) as f64 * 0.01);
        rows.insert(model, (ours, m.cost_usd));
    }
    for (model, (ours, theirs)) in rows {
        let line = match (ours, theirs) {
            (Some(o), Some(t)) if t > 0.0 => format!(
                "${o:.2} vs ${t:.2} recorded ({:+.1}%)",
                (o / t - 1.0) * 100.0
            ),
            (None, Some(t)) => format!("no price in table, ${t:.2} recorded"),
            _ => continue,
        };
        println!("    price check   {model}: {line}");
    }
}
