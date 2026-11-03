//! Feeds each synthetic fixture through the tailer, the parser, and the engine, then
//! compares the resulting machine and event list against golden YAML snapshots.
//!
//! The main transcript is scanned for its last cost-state first, as a live source would,
//! and a final `Tick` one second after the last line makes statuses deterministic.

use std::path::{Path, PathBuf};

use antfarm_core::claude::cost::cost_anchor;
use antfarm_core::claude::parse::parse_batch;
use antfarm_core::input::{AgentRef, Input};
use antfarm_core::tail::{TailConfig, Tailer};
use antfarm_core::{AgentId, Engine, EventEnvelope, ParseStats, SessionId, Snapshot};

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/synthetic")
        .join(name)
}

/// Attaches to one transcript, reads everything it has, and applies it to the engine.
fn feed(
    engine: &mut Engine,
    events: &mut Vec<EventEnvelope>,
    session: &SessionId,
    path: &Path,
    agent: AgentRef,
    max_tail: u64,
) {
    let config = TailConfig {
        max_tail,
        ..TailConfig::default()
    };
    let mut tailer = Tailer::attach(path, config).expect("fixture file opens");
    events.extend(engine.apply(Input::Attached {
        session: session.clone(),
        agent: agent.clone(),
        path: path.to_path_buf(),
        truncated_before: tailer.truncated_before(),
    }));
    if agent == AgentRef::Root
        && let Some((usd, offset, at)) = cost_anchor(path, u64::MAX).expect("scan reads")
    {
        events.extend(engine.apply(Input::CostAnchor {
            session: session.clone(),
            usd,
            offset,
            at,
        }));
    }
    let poll = tailer.poll().expect("fixture file reads");
    let mut stats = ParseStats::default();
    let (lines, errors) = parse_batch(poll.lines, &mut stats);
    events.extend(engine.apply(Input::Lines {
        session: session.clone(),
        agent,
        lines,
        errors,
        end_offset: poll.end_offset,
        mtime: None,
    }));
}

/// Runs a fixture: `main.jsonl` first, then every `subagents/agent-*.jsonl` in name order.
/// `max_tail` applies to the main transcript only.
fn run(name: &str, max_tail: u64) -> (Snapshot, Vec<EventEnvelope>) {
    let dir = fixture_dir(name);
    let session = SessionId::from(name);
    let mut engine = Engine::new();
    let mut events = Vec::new();
    feed(
        &mut engine,
        &mut events,
        &session,
        &dir.join("main.jsonl"),
        AgentRef::Root,
        max_tail,
    );
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir.join("subagents"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .collect()
        })
        .unwrap_or_default();
    subs.sort();
    for path in subs {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let id = stem.strip_prefix("agent-").unwrap_or(stem);
        feed(
            &mut engine,
            &mut events,
            &session,
            &path,
            AgentRef::Sub(AgentId::from(id)),
            u64::MAX,
        );
    }
    let last_write = engine.machine().sessions[&session]
        .last_write
        .expect("fixture lines carry timestamps");
    events.extend(engine.apply(Input::Tick {
        now: last_write + chrono::Duration::seconds(1),
    }));
    (engine.snapshot(), events)
}

fn check(name: &str, max_tail: u64) {
    let (snapshot, events) = run(name, max_tail);
    insta::assert_yaml_snapshot!(format!("{name}-machine"), snapshot.machine);
    insta::assert_yaml_snapshot!(format!("{name}-events"), events);
}

#[test]
fn parallel() {
    check("parallel", u64::MAX);
}

#[test]
fn orphan() {
    check("orphan", u64::MAX);
}

#[test]
fn truncated_tail() {
    // Land one byte inside the first line so attach skips it and the first tool_result
    // has no tool_use to pair with.
    let bytes = std::fs::read(fixture_dir("truncated-tail").join("main.jsonl")).unwrap();
    let first_nl = bytes.iter().position(|b| *b == b'\n').unwrap();
    let max_tail = bytes.len() as u64 - first_nl as u64 + 1;
    check("truncated-tail", max_tail);
}

#[test]
fn prompts_titles() {
    check("prompts-titles", u64::MAX);
}

#[test]
fn subagent_lines() {
    check("subagent-lines", u64::MAX);
}

#[test]
fn cost_with_state() {
    check("cost-with-state", u64::MAX);
}

#[test]
fn cost_without_state() {
    check("cost-without-state", u64::MAX);
}
