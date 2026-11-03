//! Proves the scrub changed nothing antfarm derives: the unscrubbed selection (in
//! memory, never written) and the scrubbed output on disk are both fed through the
//! parser and the engine the way `tests/fixtures.rs` does, and the results must match.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use antfarm_core::claude::cost::cost_anchor;
use antfarm_core::claude::parse::{Parsed, RawLine, parse_batch};
use antfarm_core::input::{AgentRef, Input};
use antfarm_core::tail::{TailConfig, Tailer};
use antfarm_core::{ContextFill, Cost, Engine, ParseStats, SessionId, Usage};
use anyhow::{Result, bail};

/// A transcript to feed: its JSONL bytes and the attach offset to report.
pub struct Feed<'a> {
    pub agent: AgentRef,
    pub bytes: &'a [u8],
    pub truncated_before: Option<u64>,
    /// Where the same bytes live on disk, when they do. The cost anchor of the root is
    /// then found by the file scan a live source would use.
    pub path: Option<&'a Path>,
}

/// What the engine derived from one run, in a form that compares exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub parse: ParseStats,
    pub open: BTreeSet<(String, String)>,
    pub agents: BTreeSet<String>,
    pub usage: Usage,
    pub agent_usage: BTreeMap<String, Usage>,
    pub cost: Cost,
    pub context: ContextFill,
    pub agent_context: BTreeMap<String, ContextFill>,
    pub calls: u64,
}

fn split_lines(bytes: &[u8]) -> Vec<RawLine> {
    let mut out = Vec::new();
    let mut offset = 0u64;
    for line in bytes.split(|b| *b == b'\n') {
        if offset as usize >= bytes.len() {
            break;
        }
        out.push(RawLine {
            offset,
            bytes: line.to_vec(),
        });
        offset += line.len() as u64 + 1;
    }
    out
}

/// The last cost-state line of an in-memory transcript, as `cost_anchor` would find it.
fn anchor_in(lines: &[RawLine]) -> Option<(f64, u64, Option<antfarm_core::Time>)> {
    let mut stats = ParseStats::default();
    for raw in lines.iter().rev() {
        if memchr::memmem::find(&raw.bytes, br#""type":"cost-state""#).is_none() {
            continue;
        }
        let (good, _) = parse_batch(vec![raw.clone()], &mut stats);
        if let Some(l) = good.first()
            && let Parsed::CostState(c) = &l.parsed
        {
            return c.total_cost_usd.map(|usd| (usd, raw.offset, c.timestamp));
        }
    }
    None
}

/// Runs the engine over `feeds` in order, root first, exactly like the fixture test.
pub fn run(session: &str, feeds: &[Feed]) -> Result<Outcome> {
    let session = SessionId::from(session);
    let mut engine = Engine::new();
    for feed in feeds {
        let lines = match feed.path {
            Some(p) => {
                let mut t = Tailer::attach(
                    p,
                    TailConfig {
                        max_tail: u64::MAX,
                        ..TailConfig::default()
                    },
                )?;
                t.poll()?.lines
            }
            None => split_lines(feed.bytes),
        };
        engine.apply(Input::Attached {
            session: session.clone(),
            agent: feed.agent.clone(),
            path: feed.path.map(Path::to_path_buf).unwrap_or_default(),
            truncated_before: feed.truncated_before,
        });
        if feed.agent == AgentRef::Root {
            let anchor = match feed.path {
                Some(p) => cost_anchor(p, u64::MAX)?,
                None => anchor_in(&lines),
            };
            if let Some((usd, offset, at)) = anchor {
                engine.apply(Input::CostAnchor {
                    session: session.clone(),
                    usd,
                    offset,
                    at,
                });
            }
        }
        let mut stats = ParseStats::default();
        let end_offset = lines
            .last()
            .map(|l| l.offset + l.bytes.len() as u64 + 1)
            .unwrap_or(0);
        let (parsed, errors) = parse_batch(lines, &mut stats);
        engine.apply(Input::Lines {
            session: session.clone(),
            agent: feed.agent.clone(),
            lines: parsed,
            errors,
            end_offset,
            mtime: None,
        });
    }
    let Some(last_write) = engine.machine().sessions[&session].last_write else {
        bail!("no line carried a timestamp");
    };
    engine.apply(Input::Tick {
        now: last_write + chrono::Duration::seconds(1),
    });
    let s = &engine.machine().sessions[&session];
    let mut open = BTreeSet::new();
    let mut agent_usage = BTreeMap::new();
    let mut agent_context = BTreeMap::new();
    let mut calls = 0;
    for (id, a) in &s.agents {
        for call in a.open.keys() {
            open.insert((id.to_string(), call.to_string()));
        }
        agent_usage.insert(id.to_string(), a.usage);
        agent_context.insert(id.to_string(), a.context);
        calls += a.calls;
    }
    Ok(Outcome {
        parse: s.parse.clone(),
        open,
        agents: s.agents.keys().map(ToString::to_string).collect(),
        usage: s.usage,
        agent_usage,
        cost: s.cost,
        context: s.context,
        agent_context,
        calls,
    })
}

/// Fails with every difference listed when the two outcomes disagree on anything but
/// byte counts.
pub fn compare(original: &Outcome, scrubbed: &Outcome) -> Result<()> {
    let mut diffs = String::new();
    let mut a = original.parse.clone();
    let mut b = scrubbed.parse.clone();
    a.bytes = 0;
    b.bytes = 0;
    if a != b {
        let _ = writeln!(diffs, "parse stats differ:\n  {a:?}\n  {b:?}");
    }
    if scrubbed.parse.errors != 0 {
        let _ = writeln!(
            diffs,
            "scrubbed output has {} parse errors: {:?}",
            scrubbed.parse.errors, scrubbed.parse.first_error
        );
    }
    if original.open != scrubbed.open {
        let _ = writeln!(
            diffs,
            "open calls differ:\n  {:?}\n  {:?}",
            original.open, scrubbed.open
        );
    }
    if original.agents != scrubbed.agents {
        let _ = writeln!(
            diffs,
            "agents differ:\n  {:?}\n  {:?}",
            original.agents, scrubbed.agents
        );
    }
    if original.usage != scrubbed.usage || original.agent_usage != scrubbed.agent_usage {
        let _ = writeln!(diffs, "usage differs");
    }
    if original.cost != scrubbed.cost {
        let _ = writeln!(
            diffs,
            "cost differs: {:?} vs {:?}",
            original.cost, scrubbed.cost
        );
    }
    if original.context != scrubbed.context || original.agent_context != scrubbed.agent_context {
        let _ = writeln!(diffs, "context differs");
    }
    if original.calls != scrubbed.calls {
        let _ = writeln!(
            diffs,
            "call counts differ: {} vs {}",
            original.calls, scrubbed.calls
        );
    }
    if diffs.is_empty() {
        Ok(())
    } else {
        bail!("verify failed:\n{diffs}");
    }
}

pub fn summary(o: &Outcome) -> String {
    let p = &o.parse;
    format!(
        "lines {} parsed {} errors {} orphans {}+{} open {} agents {} calls {} usage in/out/cr/cw {}/{}/{}/{} cost {:.4} ({:?}) ctx {} tokens{}",
        p.lines,
        p.parsed,
        p.errors,
        p.orphan_results,
        p.orphan_after_truncation,
        o.open.len(),
        o.agents.len(),
        o.calls,
        o.usage.input,
        o.usage.output,
        o.usage.cache_read,
        o.usage.cache_creation,
        o.cost.total(),
        o.cost.basis,
        o.context.tokens,
        o.context
            .pct
            .map(|p| format!(" {p:.1}%"))
            .unwrap_or_default()
    )
}
