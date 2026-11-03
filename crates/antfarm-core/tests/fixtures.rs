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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let real = root.join("real").join(name);
    if real.is_dir() {
        real
    } else {
        root.join("synthetic").join(name)
    }
}

fn sorted_files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.to_str().is_some_and(|s| s.ends_with(suffix)))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
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

/// Runs a fixture: `main.jsonl` first, then for each sub-agent in name order its meta
/// file (when present) and its transcript, then every workflow file. `max_tail` applies
/// to the main transcript only.
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
    let subs = sorted_files(&dir.join("subagents"), ".jsonl");
    for path in subs {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let id = stem.strip_prefix("agent-").unwrap_or(stem);
        let meta_path = path.with_extension("meta.json");
        if let Ok(bytes) = std::fs::read(&meta_path) {
            events.extend(engine.apply(Input::SubagentMeta {
                session: session.clone(),
                agent: AgentId::from(id),
                meta: serde_json::from_slice(&bytes).expect("meta parses"),
                transcript_mtime: None,
            }));
        }
        feed(
            &mut engine,
            &mut events,
            &session,
            &path,
            AgentRef::Sub(AgentId::from(id)),
            u64::MAX,
        );
    }
    for path in sorted_files(&dir.join("workflows"), ".json") {
        let bytes = std::fs::read(&path).expect("workflow reads");
        events.extend(engine.apply(Input::Workflow {
            session: session.clone(),
            file: serde_json::from_slice(&bytes).expect("workflow parses"),
        }));
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

/// The parts of a real fixture's result worth pinning: the agent tree, workflows, and
/// totals. The full event log of a real session is too long to review as a golden.
#[derive(serde::Serialize)]
struct Summary {
    status: antfarm_core::SessionStatus,
    events: usize,
    parse: antfarm_core::ParseStats,
    usage: antfarm_core::Usage,
    cost: antfarm_core::Cost,
    context: antfarm_core::ContextFill,
    open_calls: Vec<String>,
    files_touched: usize,
    agents: Vec<AgentSummary>,
    workflows: Vec<antfarm_core::WorkflowRun>,
}

#[derive(serde::Serialize)]
struct AgentSummary {
    id: String,
    parent: Option<String>,
    depth: u8,
    kind: String,
    background: bool,
    model: Option<String>,
    status: antfarm_core::AgentStatus,
    spawned_by: Option<String>,
    finished: bool,
    calls: u64,
    children: usize,
    cost_usd: Option<f64>,
}

fn check_real(name: &str) {
    let (snapshot, events) = run(name, u64::MAX);
    let s = &snapshot.machine.sessions[name];
    let summary = Summary {
        status: s.status,
        events: events.len(),
        parse: s.parse.clone(),
        usage: s.usage,
        cost: s.cost,
        context: s.context,
        open_calls: s
            .open_calls()
            .map(|c| format!("{} {} {}", c.agent, c.name, c.id))
            .collect(),
        files_touched: s.files_touched.len(),
        agents: s
            .agents
            .values()
            .map(|a| AgentSummary {
                id: a.id.to_string(),
                parent: a.parent.as_ref().map(ToString::to_string),
                depth: a.depth,
                kind: a.kind.clone(),
                background: a.background,
                model: a.model.clone(),
                status: a.status,
                spawned_by: a.spawned_by.as_ref().map(ToString::to_string),
                finished: a.finished.is_some(),
                calls: a.calls,
                children: a.children.len(),
                cost_usd: a.cost_usd.map(|c| (c * 1e6).round() / 1e6),
            })
            .collect(),
        workflows: s.workflows.clone(),
    };
    insta::assert_yaml_snapshot!(format!("{name}-summary"), summary);
}

#[test]
fn real_live_tail() {
    check_real("live-tail");
}

#[test]
fn real_small_session() {
    check_real("small-session");
}
