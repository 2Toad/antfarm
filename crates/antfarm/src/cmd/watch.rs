//! `antfarm watch`: a running log of what happens, or NDJSON for other tools.

use std::io::Write;
use std::time::Duration;

use antfarm_core::{Event, EventEnvelope};
use tokio_stream::StreamExt;

use super::{Scope, settle, start_live};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub scope: Scope,
    /// Emit NDJSON: `{"kind":"snapshot",...}` and `{"kind":"event",...}` lines.
    #[arg(long)]
    pub json: bool,
    /// Seconds between snapshots in --json mode; 0 for none after the first.
    #[arg(long, default_value_t = 1.0)]
    pub snapshot_every: f64,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    let live = start_live();
    // History up to now is in the first snapshot; events start after it.
    settle(&live).await;
    let mut events = Box::pin(live.handle.event_stream());
    let out = std::io::stdout();
    let names = |live: &super::Live, id: Option<&str>| -> String {
        let snap = live.handle.snapshot();
        id.and_then(|id| snap.machine.sessions.get(id))
            .and_then(|s| s.name.clone())
            .unwrap_or_else(|| id.map(|i| i.chars().take(8).collect()).unwrap_or_default())
    };

    let mut snapshots = live.handle.snapshots();
    let every = Duration::from_secs_f64(args.snapshot_every.max(0.0));
    let mut ticker = tokio::time::interval(if every.is_zero() {
        Duration::from_secs(3600)
    } else {
        every
    });
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut first = true;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            e = events.next() => {
                let Some(e) = e else { break };
                if !args.scope.keep_id(e.session.as_ref().map(|s| s.as_str())) {
                    continue;
                }
                let mut o = out.lock();
                if args.json {
                    writeln!(o, "{}", serde_json::json!({"kind": "event", "event": &*e}))?;
                } else if let Some(line) = describe(&e) {
                    let who = names(&live, e.session.as_ref().map(|s| s.as_str()));
                    writeln!(o, "{}  {:<20} {}", e.at.with_timezone(&chrono::Local).format("%H:%M:%S"), super::truncate(&who, 20), line)?;
                }
            }
            _ = ticker.tick(), if args.json => {
                let changed = snapshots.has_changed().unwrap_or(false);
                if first || (changed && !every.is_zero()) {
                    first = false;
                    let snap = snapshots.borrow_and_update().clone();
                    let sessions: Vec<_> = snap.machine.sessions.values().filter(|s| args.scope.keep(s)).collect();
                    let mut o = out.lock();
                    writeln!(o, "{}", serde_json::json!({"kind": "snapshot", "seq": snap.seq, "at": snap.at, "sessions": sessions}))?;
                }
            }
        }
    }
    live.handle.stop();
    Ok(())
}

/// One human-readable line per event worth printing.
fn describe(e: &EventEnvelope) -> Option<String> {
    Some(match &e.event {
        Event::SessionDiscovered { .. } => "session found".into(),
        Event::SessionStatusChanged { from, to } => format!("{} -> {}", from.label(), to.label()),
        Event::SessionExited => "exited".into(),
        Event::AgentSpawned {
            agent,
            kind,
            description,
            ..
        } => {
            format!(
                "spawned {kind} {} {}",
                short(agent.as_str()),
                description.as_deref().unwrap_or("")
            )
        }
        Event::AgentFinished { agent } => format!("agent {} finished", short(agent.as_str())),
        Event::ToolCallStarted {
            agent,
            name,
            summary,
            ..
        } => format!(
            "{}{name} {}",
            who(agent.as_str(), e),
            super::truncate(summary, 90)
        ),
        Event::ToolCallFinished { status, .. } if *status == antfarm_core::ResultStatus::Error => {
            "call failed".into()
        }
        Event::ToolCallFinished { .. } => return None,
        Event::CostUpdated { usd, estimate } => {
            format!("cost {}${usd:.2}", if *estimate { "~" } else { "" })
        }
        Event::Compacted { pre, post, .. } => format!(
            "compacted {} -> {} tokens",
            pre.unwrap_or(0),
            post.unwrap_or(0)
        ),
        Event::FileTruncated { .. } => "transcript rewritten, re-reading".into(),
        Event::ParseError {
            offset, message, ..
        } => format!("unreadable line at byte {offset}: {message}"),
        Event::Resync => "fell behind, some events skipped".into(),
    })
}

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Prefix for a call made by a sub-agent rather than the session's root agent.
fn who(agent: &str, e: &EventEnvelope) -> String {
    match &e.session {
        Some(s) if s.as_str() != agent => format!("[{}] ", short(agent)),
        _ => String::new(),
    }
}
