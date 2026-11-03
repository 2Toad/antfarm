//! View models: plain data built from a snapshot for one frame. Nothing here knows about
//! ratatui or reads the clock; `now` is always passed in, so every screen can be tested.

use std::collections::{HashMap, HashSet};

use antfarm_core::{
    Activity, Agent, AgentId, AgentStatus, ResultStatus, Session, SessionId, SessionStatus,
    Snapshot, Time, ToolCategory, WaitingKind,
};
use serde_json::Value;

use crate::fmt::{dur, money, project_name, rank};

/// How each state is drawn. Shape and color both follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Class {
    Waiting,
    Working,
    Finished,
    Idle,
    Still,
    Exited,
}

impl Class {
    pub fn of_session(s: SessionStatus) -> Self {
        match s {
            SessionStatus::Waiting(_) => Self::Waiting,
            SessionStatus::Working | SessionStatus::Thinking => Self::Working,
            SessionStatus::Idle => Self::Idle,
            SessionStatus::Still => Self::Still,
            SessionStatus::Exited => Self::Exited,
        }
    }

    pub fn of_agent(s: AgentStatus) -> Self {
        match s {
            AgentStatus::Waiting(_) => Self::Waiting,
            AgentStatus::Working | AgentStatus::Thinking => Self::Working,
            AgentStatus::Idle => Self::Idle,
            AgentStatus::Finished => Self::Finished,
            AgentStatus::Still => Self::Still,
            AgentStatus::Exited => Self::Exited,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Waiting => "◆",
            Self::Working => "●",
            Self::Finished | Self::Idle => "✓",
            Self::Still => "○",
            Self::Exited => "–",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub enum SortMode {
    /// Needs-you first, then longest in state.
    #[default]
    NeedsYou,
    Cost,
    Name,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            Self::NeedsYou => Self::Cost,
            Self::Cost => Self::Name,
            Self::Name => Self::NeedsYou,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs-you › time-in-state",
            Self::Cost => "cost",
            Self::Name => "name",
        }
    }
}

/// What the user has chosen, independent of where the cursor is.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Prefs {
    pub filter: String,
    pub sort: SortMode,
    pub tree: bool,
    pub show_finished: bool,
    pub snoozed: HashMap<SessionId, Time>,
    pub muted: HashSet<SessionId>,
}

impl Prefs {
    pub fn snoozed(&self, id: &SessionId, now: Time) -> bool {
        self.snoozed.get(id).is_some_and(|until| *until > now)
    }
}

pub fn session_name(s: &Session) -> String {
    s.name
        .clone()
        .unwrap_or_else(|| s.id.as_str().chars().take(8).collect())
}

pub fn agent_name(a: &Agent) -> String {
    if a.parent.is_none() {
        return "root".into();
    }
    a.description
        .clone()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| a.id.as_str().chars().take(10).collect())
}

pub fn source_label(s: &Session) -> String {
    match &s.source {
        Some(src) => match &src.tmux_pane {
            Some(p) => format!("tmux:{p}"),
            None if src.app == "Visual Studio Code" => "VS Code".into(),
            None => src.app.clone(),
        },
        None if s.entrypoint.as_deref() == Some("claude-vscode") => "VS Code".into(),
        None => "-".into(),
    }
}

/// Short state word for a table cell.
pub fn state_word(status: SessionStatus, open_category: Option<ToolCategory>) -> &'static str {
    match status {
        SessionStatus::Waiting(WaitingKind::Permission) => "PERMIT",
        SessionStatus::Waiting(WaitingKind::Question) => "ASK",
        SessionStatus::Working => match open_category {
            Some(ToolCategory::Spawn) => "spawn",
            Some(ToolCategory::Shell) => "run",
            _ => "tool",
        },
        SessionStatus::Thinking => "think",
        SessionStatus::Idle => "idle",
        SessionStatus::Still => "still",
        SessionStatus::Exited => "exited",
    }
}

fn agent_word(a: &Agent) -> &'static str {
    match a.status {
        AgentStatus::Waiting(WaitingKind::Permission) => "PERMIT",
        AgentStatus::Waiting(WaitingKind::Question) => "ASK",
        AgentStatus::Working => match latest_open(a).map(|c| c.category) {
            Some(ToolCategory::Spawn) => "spawn",
            Some(ToolCategory::Shell) => "run",
            _ => "tool",
        },
        AgentStatus::Thinking => "think",
        AgentStatus::Idle => "idle",
        AgentStatus::Finished => "done",
        AgentStatus::Still => "still",
        AgentStatus::Exited => "exited",
    }
}

fn latest_open(a: &Agent) -> Option<&antfarm_core::ToolCall> {
    a.open.values().max_by_key(|c| c.started)
}

/// The call an agent is on, else its most recent one, as `(tool, target)`.
pub fn agent_current(a: &Agent) -> (String, String) {
    if let Some(c) = latest_open(a) {
        return (c.name.clone(), c.summary.clone());
    }
    a.activity
        .iter()
        .rev()
        .find_map(|x| match x {
            Activity::Call { name, summary, .. } => Some((name.clone(), summary.clone())),
            _ => None,
        })
        .unwrap_or_default()
}

/// The open call that can be blocking the session: the newest one on the agent chain
/// from the root through open foreground spawns. Background agents never prompt.
pub fn blocking_call(s: &Session) -> Option<&antfarm_core::ToolCall> {
    let leaf = antfarm_core::engine::status::leaf_agent(&s.agents, &s.root);
    s.agents.get(leaf).and_then(latest_open)
}

/// The call the session is blocked on or busy with: the leaf agent's open call first,
/// then any open call, then the most recent call.
pub fn session_current(s: &Session) -> (String, String) {
    if let Some(c) = blocking_call(s) {
        return (c.name.clone(), c.summary.clone());
    }
    // A waiting session with nothing open on its prompting chain is waiting on what the
    // registry says, not on some background agent's call.
    if s.status.needs_human() {
        let label = s
            .registry
            .as_ref()
            .and_then(|r| r.waiting_for.clone())
            .unwrap_or_else(|| "waiting".into());
        return (String::new(), label);
    }
    let mut open: Vec<_> = s.open_calls().collect();
    open.sort_by_key(|c| c.started);
    if let Some(c) = open.last() {
        return (c.name.clone(), c.summary.clone());
    }
    let latest = s
        .agents
        .values()
        .flat_map(|a| a.activity.iter())
        .filter_map(|x| match x {
            Activity::Call {
                name, summary, at, ..
            } => Some((*at, name, summary)),
            _ => None,
        })
        .max_by_key(|(at, ..)| *at);
    latest
        .map(|(_, n, s)| (n.clone(), s.clone()))
        .unwrap_or_default()
}

/// Minutes `[now - 9 ..= now]` of calls for a sparkline, oldest first.
pub fn spark(agents: &[&Agent], now: Time) -> [u32; 10] {
    let mut out = [0u32; 10];
    let base = minute(now);
    for a in agents {
        for b in &a.minutes {
            let back = (base - b.start).num_minutes();
            if (0..10).contains(&back) {
                out[9 - back as usize] += b.calls;
            }
        }
    }
    out
}

/// New tokens per minute over the last five minutes.
pub fn tok_per_min(agents: &[&Agent], now: Time) -> f64 {
    let base = minute(now);
    let total: u64 = agents
        .iter()
        .flat_map(|a| a.minutes.iter())
        .filter(|b| (0..5).contains(&(base - b.start).num_minutes()))
        .map(|b| b.tokens)
        .sum();
    total as f64 / 5.0
}

fn minute(t: Time) -> Time {
    t - chrono::Duration::seconds(t.timestamp().rem_euclid(60))
        - chrono::Duration::nanoseconds(t.timestamp_subsec_nanos() as i64)
}

fn secs_since(t: Option<Time>, now: Time) -> Option<i64> {
    t.map(|t| (now - t).num_seconds().max(0))
}

// ---------------------------------------------------------------------------------------
// Machine screen

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Meters {
    pub sessions: usize,
    pub agents: usize,
    pub waiting: usize,
    pub oldest_wait: Option<i64>,
    pub working: usize,
    pub finished: usize,
    pub still: usize,
    pub exited: usize,
    pub calls_per_min: f64,
    pub tok_per_min: f64,
    /// Lifetime cost of the sessions written to today. Per-day cost is not recorded, so
    /// this is not money spent today.
    pub spent_today: f64,
    pub apps: Vec<String>,
}

pub fn meters(snap: &Snapshot, now: Time) -> Meters {
    let sessions: Vec<&Session> = snap.machine.sessions.values().collect();
    let agents: Vec<&Agent> = sessions.iter().flat_map(|s| s.agents.values()).collect();
    let mut waits: Vec<i64> = sessions
        .iter()
        .filter(|s| s.status.needs_human())
        .filter_map(|s| secs_since(s.status_since, now))
        .collect();
    waits.sort();
    let today = now.date_naive();
    let calls: u32 = {
        let base = minute(now);
        agents
            .iter()
            .flat_map(|a| a.minutes.iter())
            .filter(|b| (0..5).contains(&(base - b.start).num_minutes()))
            .map(|b| b.calls)
            .sum()
    };
    let mut apps: Vec<String> = sessions
        .iter()
        .filter(|s| s.status != SessionStatus::Exited)
        .map(|s| source_label(s))
        .filter(|a| a != "-")
        .collect();
    apps.sort();
    apps.dedup();
    Meters {
        sessions: sessions
            .iter()
            .filter(|s| s.status != SessionStatus::Exited)
            .count(),
        agents: agents.len(),
        waiting: sessions.iter().filter(|s| s.status.needs_human()).count(),
        oldest_wait: waits.last().copied(),
        working: agents
            .iter()
            .filter(|a| Class::of_agent(a.status) == Class::Working)
            .count(),
        finished: agents
            .iter()
            .filter(|a| a.status == AgentStatus::Finished)
            .count(),
        still: sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Still)
            .count(),
        exited: sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Exited)
            .count(),
        calls_per_min: calls as f64 / 5.0,
        tok_per_min: tok_per_min(&agents, now),
        spent_today: sessions
            .iter()
            .filter(|s| s.last_write.is_some_and(|t| t.date_naive() == today))
            .map(|s| s.cost.total())
            .sum(),
        apps,
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NeedsYou {
    pub session: SessionId,
    pub name: String,
    pub project: String,
    pub kind: WaitingKind,
    pub request: String,
    /// The whole command a shell call is waiting to run, for copying.
    pub command: Option<String>,
    pub source: String,
    pub age: Option<i64>,
}

/// Waiting sessions, oldest first, minus snoozed ones.
pub fn needs_you(snap: &Snapshot, prefs: &Prefs, now: Time) -> Vec<NeedsYou> {
    let mut out: Vec<NeedsYou> = snap
        .machine
        .sessions
        .values()
        .filter(|s| !prefs.snoozed(&s.id, now))
        .filter_map(|s| {
            let SessionStatus::Waiting(kind) = s.status else {
                return None;
            };
            let (tool, target) = blocking_call(s)
                .map(|c| (c.name.clone(), c.summary.clone()))
                .unwrap_or_default();
            Some(NeedsYou {
                session: s.id.clone(),
                name: session_name(s),
                project: project_name(s),
                kind,
                request: if tool.is_empty() {
                    s.registry
                        .as_ref()
                        .and_then(|r| r.waiting_for.clone())
                        .unwrap_or_default()
                } else if tool == "AskUserQuestion" {
                    target
                } else {
                    format!("{tool} {target}")
                },
                command: blocking_call(s)
                    .and_then(|c| c.input.get("command"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                source: source_label(s),
                age: secs_since(s.status_since, now),
            })
        })
        .collect();
    out.sort_by_key(|x| std::cmp::Reverse(x.age));
    out
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Row {
    pub session: SessionId,
    pub agent: Option<AgentId>,
    pub depth: u8,
    pub class: Class,
    pub pid: Option<u32>,
    pub name: String,
    pub project: String,
    pub branch: String,
    pub source: String,
    pub state: &'static str,
    pub for_secs: Option<i64>,
    pub tool: String,
    pub target: String,
    pub calls: u64,
    pub tok_per_min: Option<f64>,
    pub cost: String,
    pub ctx: Option<f32>,
    pub spark: [u32; 10],
    pub muted: bool,
}

fn matches(prefs: &Prefs, s: &Session, row_text: &[&str], state: &str) -> bool {
    prefs.filter.split_whitespace().all(|term| {
        let term = term.to_lowercase();
        if let Some(v) = term.strip_prefix("state:") {
            return state.to_lowercase().starts_with(v)
                || Class::of_session(s.status) == class_named(v).unwrap_or(Class::Exited)
                    && class_named(v).is_some();
        }
        if let Some(v) = term.strip_prefix("project:") {
            return project_name(s).to_lowercase().contains(v);
        }
        row_text.iter().any(|t| t.to_lowercase().contains(&term))
    })
}

fn class_named(v: &str) -> Option<Class> {
    Some(match v {
        "waiting" | "wait" => Class::Waiting,
        "working" | "work" => Class::Working,
        "idle" => Class::Idle,
        "still" => Class::Still,
        "exited" => Class::Exited,
        _ => return None,
    })
}

/// Table rows: one per session, then its live sub-agents when the tree is on.
pub fn table_rows(snap: &Snapshot, prefs: &Prefs, show_exited: bool, now: Time) -> Vec<Row> {
    let mut sessions: Vec<&Session> = snap
        .machine
        .sessions
        .values()
        .filter(|s| show_exited || s.status != SessionStatus::Exited)
        .collect();
    sessions.sort_by(|a, b| {
        let muted = prefs
            .muted
            .contains(&a.id)
            .cmp(&prefs.muted.contains(&b.id));
        let key = match prefs.sort {
            SortMode::NeedsYou => rank(a)
                .cmp(&rank(b))
                .then(a.status_since.cmp(&b.status_since)),
            SortMode::Cost => b.cost.total().total_cmp(&a.cost.total()),
            SortMode::Name => session_name(a)
                .to_lowercase()
                .cmp(&session_name(b).to_lowercase()),
        };
        muted.then(key).then(a.id.cmp(&b.id))
    });
    let mut rows = Vec::new();
    for s in sessions {
        let (tool, target) = session_current(s);
        let open_cat = s.open_calls().max_by_key(|c| c.started).map(|c| c.category);
        let state = state_word(s.status, open_cat);
        let name = session_name(s);
        let project = project_name(s);
        let branch = s.branch.clone().unwrap_or_default();
        if !matches(prefs, s, &[&name, &project, &branch, &tool, &target], state) {
            continue;
        }
        let agents: Vec<&Agent> = s.agents.values().collect();
        rows.push(Row {
            session: s.id.clone(),
            agent: None,
            depth: 0,
            class: Class::of_session(s.status),
            pid: s.pid,
            name,
            project: project.clone(),
            branch,
            source: source_label(s),
            state,
            for_secs: secs_since(s.status_since, now),
            tool,
            target,
            calls: agents.iter().map(|a| a.calls).sum(),
            tok_per_min: Some(tok_per_min(&agents, now)),
            cost: money(&s.cost),
            ctx: s.context.pct,
            spark: spark(&agents, now),
            muted: prefs.muted.contains(&s.id),
        });
        if !prefs.tree {
            continue;
        }
        for a in tree_order(s) {
            if a.parent.is_none() {
                continue;
            }
            let live = matches!(Class::of_agent(a.status), Class::Working | Class::Waiting);
            if !live
                && !(prefs.show_finished
                    && a.status == AgentStatus::Finished
                    && a.finished.is_some_and(|f| (now - f).num_minutes() < 10))
            {
                continue;
            }
            let (tool, target) = agent_current(a);
            rows.push(Row {
                session: s.id.clone(),
                agent: Some(a.id.clone()),
                depth: a.depth.max(1),
                class: Class::of_agent(a.status),
                pid: None,
                name: agent_name(a),
                project: project.clone(),
                branch: String::new(),
                source: String::new(),
                state: agent_word(a),
                for_secs: secs_since(a.last_write.or(a.started), now),
                tool,
                target,
                calls: a.calls,
                tok_per_min: Some(tok_per_min(&[a], now)),
                cost: a
                    .cost_usd
                    .map(|c| format!("${c:.2}"))
                    .unwrap_or_else(|| "-".into()),
                ctx: a.context.pct,
                spark: spark(&[a], now),
                muted: prefs.muted.contains(&s.id),
            });
        }
    }
    rows
}

/// The session's agents depth first, children in spawn order.
pub fn tree_order(s: &Session) -> Vec<&Agent> {
    let mut out = Vec::new();
    let mut stack = vec![&s.root];
    while let Some(id) = stack.pop() {
        let Some(a) = s.agents.get(id) else { continue };
        out.push(a);
        for c in a.children.iter().rev() {
            stack.push(c);
        }
    }
    // Agents not reachable from the root still appear, at the end.
    for a in s.agents.values() {
        if !out.iter().any(|x| x.id == a.id) {
            out.push(a);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Session screen

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Waiting {
    pub kind: WaitingKind,
    pub agent: String,
    pub tool: String,
    /// The command for a shell call, the question for AskUserQuestion, else the summary.
    pub request: String,
    pub reason: Option<String>,
    pub touches: Vec<String>,
    pub asked_at: Option<Time>,
    pub after: Option<String>,
    pub age: Option<i64>,
}

/// What the session is waiting on, from its leaf open call.
pub fn waiting(s: &Session, now: Time) -> Option<Waiting> {
    let SessionStatus::Waiting(kind) = s.status else {
        return None;
    };
    let call = blocking_call(s);
    let Some(call) = call else {
        return Some(Waiting {
            kind,
            agent: "root".into(),
            tool: String::new(),
            request: s
                .registry
                .as_ref()
                .and_then(|r| r.waiting_for.clone())
                .unwrap_or_else(|| "waiting".into()),
            reason: None,
            touches: Vec::new(),
            asked_at: s.status_since,
            after: None,
            age: secs_since(s.status_since, now),
        });
    };
    let input = &*call.input;
    let request = match call.name.as_str() {
        "AskUserQuestion" => call.summary.clone(),
        _ => input
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| call.summary.clone()),
    };
    let agent = s.agents.get(&call.agent);
    let after = agent.and_then(|a| {
        let mut seen = false;
        a.activity.iter().rev().find_map(|x| match x {
            Activity::Call {
                call: id,
                name,
                summary,
                ..
            } => {
                if seen {
                    return Some(format!("{name} {summary}"));
                }
                seen = *id == call.id;
                None
            }
            _ => None,
        })
    });
    Some(Waiting {
        kind,
        agent: agent.map(agent_name).unwrap_or_else(|| "root".into()),
        tool: call.name.clone(),
        request,
        reason: input
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        touches: touched_paths(input),
        asked_at: call.started,
        after,
        age: secs_since(call.started.or(s.status_since), now),
    })
}

/// File paths a tool input names: `file_path`, `notebook_path`, and path-like words in a
/// command.
pub fn touched_paths(input: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in ["file_path", "notebook_path", "path"] {
        if let Some(p) = input.get(k).and_then(Value::as_str) {
            out.push(p.to_owned());
        }
    }
    if let Some(cmd) = input.get("command").and_then(Value::as_str) {
        for w in
            cmd.split(|c: char| c.is_whitespace() || c == '=' || c == ';' || c == '"' || c == '\'')
        {
            let looks_like_path = w.contains('/')
                && w.rsplit('/').next().is_some_and(|last| last.contains('.'))
                && !w.starts_with("http")
                && !w.starts_with('-');
            if looks_like_path && !out.iter().any(|o| o == w) {
                out.push(w.to_owned());
            }
        }
    }
    out.truncate(6);
    out
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AgentLine {
    pub id: AgentId,
    pub depth: u8,
    pub name: String,
    pub class: Class,
    pub state: &'static str,
    pub detail: String,
}

pub fn agent_lines(s: &Session, now: Time) -> Vec<AgentLine> {
    tree_order(s)
        .into_iter()
        .map(|a| {
            let elapsed = match (a.started, a.finished) {
                (Some(st), Some(f)) => dur((f - st).num_seconds()),
                (Some(st), None) => dur((now - st).num_seconds()),
                _ => "-".into(),
            };
            AgentLine {
                id: a.id.clone(),
                depth: a.depth,
                name: if a.parent.is_none() {
                    session_name(s)
                } else {
                    agent_name(a)
                },
                class: Class::of_agent(a.status),
                state: agent_word(a),
                detail: format!("{elapsed} · {} calls", a.calls),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Lane {
    pub label: String,
    pub depth: u8,
    /// `(start, end, class)` as fractions of the session span.
    pub segments: Vec<(f64, f64, Class)>,
}

/// The longest stretch a timeline covers.
pub const LANE_HOURS: i64 = 2;

/// One lane per agent across the session's span: when each ran, and the root's waiting
/// tail when the session is blocked.
pub fn lanes(s: &Session, now: Time) -> (Option<(Time, Time)>, Vec<Lane>) {
    let start = s.agents.values().filter_map(|a| a.started).min();
    let Some(start) = start else {
        return (None, Vec::new());
    };
    // A long-lived session would squash recent work into a sliver; show the last two
    // hours at most.
    let start = start.max(now - chrono::Duration::hours(LANE_HOURS));
    let end = now.max(start + chrono::Duration::seconds(1));
    let span = (end - start).num_milliseconds() as f64;
    let frac = |t: Time| ((t - start).num_milliseconds() as f64 / span).clamp(0.0, 1.0);
    let lanes = tree_order(s)
        .into_iter()
        .filter(|a| a.started.is_some())
        // Agents that ended before the window would be a one-cell smear at its edge.
        .filter(|a| {
            a.parent.is_none()
                || a.finished
                    .or(a.last_write)
                    .or(a.started)
                    .is_some_and(|e| e >= start)
        })
        .take(12)
        .map(|a| {
            let st = frac(a.started.unwrap());
            let mut segments = Vec::new();
            let class = Class::of_agent(a.status);
            let stop = a.finished.or(match class {
                Class::Still | Class::Exited | Class::Idle => a.last_write,
                _ => None,
            });
            match (class, a.parent.is_none(), s.status) {
                (_, true, SessionStatus::Waiting(_)) => {
                    let w = s.status_since.map(frac).unwrap_or(1.0).max(st);
                    segments.push((st, w, Class::Working));
                    segments.push((w, 1.0, Class::Waiting));
                }
                _ => segments.push((
                    st,
                    stop.map(frac).unwrap_or(1.0).max(st),
                    if a.finished.is_some() {
                        Class::Finished
                    } else {
                        class
                    },
                )),
            }
            Lane {
                label: if a.parent.is_none() {
                    session_name(s)
                } else {
                    agent_name(a)
                },
                depth: a.depth,
                segments,
            }
        })
        .collect();
    (Some((start, end)), lanes)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CallLine {
    pub agent: AgentId,
    pub call: antfarm_core::ToolCallId,
    pub agent_name: String,
    pub at: Option<Time>,
    pub tool: String,
    pub target: String,
    pub status: ResultStatus,
    pub elapsed: Option<i64>,
    pub waiting: bool,
}

/// Calls across the session's agents, newest first. Each agent's history is already in
/// time order, so this merges from the newest end and builds only `limit` rows, which
/// keeps it cheap for sessions with hundreds of agents.
pub fn call_lines(s: &Session, limit: usize) -> Vec<CallLine> {
    call_lines_before(s, limit, None)
}

/// Like [`call_lines`], keeping only calls that started at or before `before`.
pub fn call_lines_before(s: &Session, limit: usize, before: Option<Time>) -> Vec<CallLine> {
    use std::collections::BinaryHeap;
    let waiting_call = match s.status {
        SessionStatus::Waiting(_) => blocking_call(s).map(|c| c.id.clone()),
        _ => None,
    };
    let agents: Vec<&Agent> = s.agents.values().collect();
    let prev_call = |a: &Agent, before: usize| {
        (0..before)
            .rev()
            .find(|&i| matches!(a.activity[i], Activity::Call { .. }))
    };
    let at_of = |a: &Agent, i: usize| match &a.activity[i] {
        Activity::Call { at, .. } => *at,
        _ => None,
    };
    let mut heap = BinaryHeap::new();
    for (ai, a) in agents.iter().enumerate() {
        if let Some(i) = prev_call(a, a.activity.len()) {
            heap.push((at_of(a, i), ai, i));
        }
    }
    let mut out = Vec::with_capacity(limit.min(256));
    while out.len() < limit {
        let Some((at, ai, i)) = heap.pop() else { break };
        let a = agents[ai];
        let too_late = matches!((at, before), (Some(at), Some(b)) if at > b);
        if too_late {
            if let Some(j) = prev_call(a, i) {
                heap.push((at_of(a, j), ai, j));
            }
            continue;
        }
        if let Activity::Call {
            call,
            name: tool,
            summary,
            at,
            finished,
            status,
            ..
        } = &a.activity[i]
        {
            out.push(CallLine {
                agent: a.id.clone(),
                call: call.clone(),
                agent_name: if a.parent.is_none() {
                    session_name(s)
                } else {
                    agent_name(a)
                },
                at: *at,
                tool: tool.clone(),
                target: summary.clone(),
                status: *status,
                elapsed: match (at, finished) {
                    (Some(a), Some(f)) => Some((*f - *a).num_milliseconds()),
                    _ => None,
                },
                waiting: waiting_call.as_ref() == Some(call),
            });
        }
        if let Some(j) = prev_call(a, i) {
            heap.push((at_of(a, j), ai, j));
        }
    }
    out
}

/// Every call the session's agents have made, including ones aged out of history.
pub fn call_count(s: &Session) -> u64 {
    s.agents.values().map(|a| a.calls).sum()
}

// ---------------------------------------------------------------------------------------
// Agent and call screens

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Waterfall {
    pub first: usize,
    pub count: usize,
    pub tool: String,
    pub target: String,
    pub at: Option<Time>,
    pub elapsed_ms: Option<i64>,
    pub status: ResultStatus,
    pub call: antfarm_core::ToolCallId,
}

/// An agent's calls oldest first, with runs of the same tool on the same first word of
/// target collapsed into one row.
pub fn waterfall(a: &Agent) -> Vec<Waterfall> {
    let mut out: Vec<Waterfall> = Vec::new();
    let mut n = 0;
    for x in &a.activity {
        let Activity::Call {
            call,
            name,
            summary,
            at,
            finished,
            status,
            ..
        } = x
        else {
            continue;
        };
        n += 1;
        let elapsed_ms = match (at, finished) {
            (Some(a), Some(f)) => Some((*f - *a).num_milliseconds()),
            _ => None,
        };
        let key = summary.split_whitespace().next().unwrap_or("");
        if let Some(last) = out.last_mut()
            && last.tool == *name
            && last.status == *status
            && *status == ResultStatus::Ok
            && last.target.split_whitespace().next().unwrap_or("") == key
            && !key.is_empty()
        {
            last.count += 1;
            last.elapsed_ms = match (last.elapsed_ms, elapsed_ms) {
                (Some(x), Some(y)) => Some(x + y),
                (x, y) => x.or(y),
            };
            last.call = call.clone();
            continue;
        }
        out.push(Waterfall {
            first: n,
            count: 1,
            tool: name.clone(),
            target: summary.clone(),
            at: *at,
            elapsed_ms,
            status: *status,
            call: call.clone(),
        });
    }
    out
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CallDetail {
    pub tool: String,
    pub category: ToolCategory,
    pub summary: String,
    pub status: ResultStatus,
    pub started: Option<Time>,
    pub finished: Option<Time>,
    pub elapsed: Option<i64>,
    pub background: bool,
    /// Pretty-printed input with secrets masked, or `None` when it is not kept.
    pub input: Option<String>,
    /// The assistant's words just before the call, when still kept.
    pub why: Option<String>,
    /// The background command this call started, if any.
    pub background_run: Option<antfarm_core::BackgroundRun>,
}

pub fn call_detail(a: &Agent, id: &antfarm_core::ToolCallId, now: Time) -> Option<CallDetail> {
    a.activity.iter().rev().find_map(|x| match x {
        Activity::Call {
            call,
            name,
            summary,
            at,
            finished,
            status,
            input,
            why,
            background: background_run,
        } if call == id => {
            let input_val = input
                .as_deref()
                .or_else(|| a.open.get(id).map(|c| &*c.input));
            Some(CallDetail {
                tool: name.clone(),
                category: ToolCategory::of(name),
                summary: summary.clone(),
                status: *status,
                started: *at,
                finished: *finished,
                elapsed: at.map(|t| (finished.unwrap_or(now) - t).num_seconds()),
                background: input_val
                    .and_then(|v| v.get("run_in_background"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                input: input_val
                    .map(|v| mask_secrets(&serde_json::to_string_pretty(v).unwrap_or_default())),
                why: why.as_deref().map(mask_secrets),
                background_run: background_run.clone(),
            })
        }
        _ => None,
    })
}

/// Masks common secret shapes in text: known key prefixes, the value after a bearer or
/// basic auth scheme, the value of an assignment whose key names a secret, passwords in
/// URLs and `-u user:pass`, and long dense tokens. Masked values keep their first four
/// characters.
pub fn mask_secrets(text: &str) -> String {
    const PREFIXES: &[&str] = &[
        "sk-",
        "sk_live_",
        "sk_test_",
        "rk_live_",
        "ghp_",
        "gho_",
        "ghs_",
        "ghu_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "AKIA",
        "ASIA",
        "AIza",
        "glpat-",
        "npm_",
        "hf_",
        "pypi-",
    ];
    let text = mask_url_passwords(text);
    let mut out = String::with_capacity(text.len());
    // How strongly the next word is suspected: 0 not at all, 1 after a secret key or
    // bearer, 2 after `basic`, which is also an English word and so needs a long value.
    let mut next = 0u8;
    let mut user_flag = false;
    let mut word = String::new();
    // The last whole word, so `"api_key": "x"` is caught across the closing quote.
    let mut prev = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphanumeric() || "-_+/.".contains(c) {
            word.push(c);
        } else {
            let key = if word.is_empty() {
                prev.as_str()
            } else {
                word.as_str()
            };
            let assignment = (c == '=' || c == ':') && names_secret(key);
            // `curl -u admin:password`: the part after the colon.
            let user_pass = c == ':' && !word.is_empty() && user_flag;
            flush_word(
                &mut word,
                &mut out,
                &mut next,
                &mut prev,
                &mut user_flag,
                PREFIXES,
            );
            out.push(c);
            if assignment || user_pass {
                next = 1;
                // Skip spaces and an opening quote after `KEY:` so the value is masked.
                while i + 1 < chars.len() && matches!(chars[i + 1], ' ' | '"' | '\'') {
                    i += 1;
                    out.push(chars[i]);
                }
            }
        }
        i += 1;
    }
    flush_word(
        &mut word,
        &mut out,
        &mut next,
        &mut prev,
        &mut user_flag,
        PREFIXES,
    );
    out
}

fn flush_word(
    word: &mut String,
    out: &mut String,
    next: &mut u8,
    prev: &mut String,
    user_flag: &mut bool,
    prefixes: &[&str],
) {
    if word.is_empty() {
        return;
    }
    let n = word.chars().count();
    let numeric = word
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '_');
    let suspected = match *next {
        1 => n >= 6 && !numeric,
        2 => n >= 12 && !numeric,
        _ => false,
    };
    let secret = suspected
        || prefixes
            .iter()
            .any(|p| word.starts_with(p) && n >= p.len() + 8)
        || dense_token(word);
    if secret {
        // By characters, not bytes: values can start with any script.
        out.extend(word.chars().take(4));
        out.push_str("••••••••");
    } else {
        out.push_str(word);
    }
    *next = if word.eq_ignore_ascii_case("bearer") {
        1
    } else if word.eq_ignore_ascii_case("basic") {
        2
    } else {
        0
    };
    // `-u user:pass`: the flag is the word right before the user name.
    *user_flag = matches!(word.as_str(), "-u" | "--user");
    *prev = std::mem::take(word);
}

/// Key names that hold secrets: whole words or `_`, `-`, `.` suffixed parts, so
/// `api_key`, `GITHUB_TOKEN`, and `password` match but `max_tokens`, `tokenizer`,
/// `keyboard`, and `--key-file` do not.
fn names_secret(key: &str) -> bool {
    const NAMES: &[&str] = &[
        "key",
        "token",
        "password",
        "passwd",
        "pwd",
        "secret",
        "apikey",
        "credentials",
        "credential",
    ];
    let k = key.trim_start_matches('-').to_ascii_lowercase();
    NAMES.iter().any(|n| {
        k == *n
            || b"_-.".iter().any(|sep| {
                k.len() > n.len() && k.ends_with(n) && k.as_bytes()[k.len() - n.len() - 1] == *sep
            })
    })
}

/// A long token with no separators and enough digits among both cases to look random,
/// unlike a camelCase identifier.
fn dense_token(w: &str) -> bool {
    let n = w.len();
    let digits = w.bytes().filter(u8::is_ascii_digit).count();
    n >= 32
        && w.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+=_".contains(&b))
        && w.bytes().any(|b| b.is_ascii_uppercase())
        && w.bytes().any(|b| b.is_ascii_lowercase())
        && digits * 100 / n >= 15
}

/// `scheme://user:password@host` keeps the user and masks the password.
fn mask_url_passwords(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '"' | '\''))
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match (authority.find(':'), authority.rfind('@')) {
            (Some(colon), Some(at)) if at > colon + 1 => {
                out.push_str(&authority[..=colon]);
                out.push_str("••••••••");
                out.push_str(&authority[at..]);
            }
            _ => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

pub fn ms(ms: Option<i64>) -> String {
    match ms {
        None => "-".into(),
        Some(m) if m < 1000 => format!("{:.1}s", m as f64 / 1000.0),
        Some(m) if m < 60_000 => format!("{:.1}s", m as f64 / 1000.0),
        Some(m) => dur(m / 1000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use antfarm_core::{Machine, ToolCall, ToolCallId};
    use std::sync::Arc;

    fn t(s: &str) -> Time {
        s.parse().unwrap()
    }

    fn call(agent: &AgentId, id: &str, name: &str, input: Value, at: &str) -> ToolCall {
        let input_for_summary = input.clone();
        ToolCall {
            id: ToolCallId::from(id),
            agent: agent.clone(),
            name: name.into(),
            category: ToolCategory::of(name),
            input: Arc::new(input),
            summary: antfarm_core::claude::reduce::summarize(name, &input_for_summary),
            started: Some(t(at)),
            finished: None,
            status: ResultStatus::Pending,
            result_bytes: 0,
            parallel_group: None,
        }
    }

    fn session(id: &str, name: &str, status: SessionStatus, since: &str) -> Session {
        let mut s = Session::new(SessionId::from(id), format!("/w/{name}").into());
        s.name = Some(name.into());
        s.status = status;
        s.status_since = Some(t(since));
        s.last_write = Some(t(since));
        s
    }

    fn snap(sessions: Vec<Session>) -> Snapshot {
        let mut m = Machine::default();
        for s in sessions {
            m.sessions.insert(s.id.clone(), s);
        }
        Snapshot {
            seq: 1,
            at: t("2026-11-03T12:00:00Z"),
            machine: m,
        }
    }

    #[test]
    fn needs_you_is_oldest_first_and_honors_snooze() {
        let now = t("2026-11-03T12:00:00Z");
        let mut a = session(
            "a",
            "api",
            SessionStatus::Waiting(WaitingKind::Permission),
            "2026-11-03T11:58:00Z",
        );
        let root = a.root.clone();
        let c = call(
            &root,
            "t1",
            "Bash",
            serde_json::json!({"command": "pnpm db:migrate --env=dev", "description": "apply migration"}),
            "2026-11-03T11:58:00Z",
        );
        a.agents[&root].open.insert(c.id.clone(), c);
        let b = session(
            "b",
            "web",
            SessionStatus::Waiting(WaitingKind::Question),
            "2026-11-03T11:59:30Z",
        );
        let c2 = session("c", "docs", SessionStatus::Working, "2026-11-03T11:00:00Z");
        let s = snap(vec![b, a, c2]);
        let mut prefs = Prefs::default();
        let n = needs_you(&s, &prefs, now);
        assert_eq!(
            n.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            ["api", "web"]
        );
        assert_eq!(n[0].request, "Bash pnpm db:migrate --env=dev");
        assert_eq!(n[0].age, Some(120));
        prefs
            .snoozed
            .insert(SessionId::from("a"), now + chrono::Duration::minutes(10));
        assert_eq!(needs_you(&s, &prefs, now).len(), 1);
        assert_eq!(
            needs_you(&s, &prefs, now + chrono::Duration::minutes(11)).len(),
            2
        );

        let w = waiting(&s.machine.sessions["a"], now).unwrap();
        assert_eq!(w.request, "pnpm db:migrate --env=dev");
        assert_eq!(w.reason.as_deref(), Some("apply migration"));
    }

    #[test]
    fn rows_sort_filter_and_tree() {
        let now = t("2026-11-03T12:00:00Z");
        let mut a = session("a", "api", SessionStatus::Working, "2026-11-03T11:50:00Z");
        let root = a.root.clone();
        let mut sub = Agent::new(AgentId::from("a1"), "Explore");
        sub.parent = Some(root.clone());
        sub.depth = 1;
        sub.status = AgentStatus::Working;
        sub.description = Some("map callers".into());
        a.agents.insert(sub.id.clone(), sub);
        a.agents[&root].children.push(AgentId::from("a1"));
        a.cost.reported_usd = Some(4.0);
        a.cost.basis = antfarm_core::CostBasis::Reported;
        let b = session(
            "b",
            "web",
            SessionStatus::Waiting(WaitingKind::Question),
            "2026-11-03T11:59:00Z",
        );
        let mut c = session("c", "old", SessionStatus::Exited, "2026-11-03T10:00:00Z");
        c.cost.reported_usd = Some(9.0);
        let s = snap(vec![a, b, c]);
        let mut prefs = Prefs {
            tree: true,
            ..Prefs::default()
        };
        let names = |rows: &[Row]| rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(&table_rows(&s, &prefs, false, now)),
            ["web", "api", "map callers"]
        );
        assert_eq!(
            names(&table_rows(&s, &prefs, true, now)),
            ["web", "api", "map callers", "old"]
        );
        prefs.tree = false;
        prefs.sort = SortMode::Cost;
        assert_eq!(
            names(&table_rows(&s, &prefs, true, now)),
            ["old", "api", "web"]
        );
        prefs.filter = "state:wait".into();
        assert_eq!(names(&table_rows(&s, &prefs, true, now)), ["web"]);
        prefs.filter = "project:ap".into();
        assert_eq!(names(&table_rows(&s, &prefs, true, now)), ["api"]);
        prefs.filter = String::new();
        prefs.muted.insert(SessionId::from("web"));
        prefs.muted.insert(SessionId::from("b"));
        prefs.sort = SortMode::NeedsYou;
        assert_eq!(
            names(&table_rows(&s, &prefs, false, now))
                .last()
                .map(String::as_str),
            Some("web"),
            "muted sorts last"
        );
    }

    #[test]
    fn sparkline_and_rates_from_minute_buckets() {
        let now = t("2026-11-03T12:00:30Z");
        let mut a = Agent::new(AgentId::from("x"), "root");
        a.bump_minute(Some(t("2026-11-03T12:00:10Z")), 3, 500);
        a.bump_minute(Some(t("2026-11-03T11:58:10Z")), 1, 500);
        a.bump_minute(Some(t("2026-11-03T11:40:00Z")), 9, 9000);
        let sp = spark(&[&a], now);
        assert_eq!(sp[9], 3);
        assert_eq!(sp[7], 1);
        assert_eq!(sp.iter().sum::<u32>(), 4);
        assert_eq!(tok_per_min(&[&a], now), 200.0);
    }

    #[test]
    fn touched_paths_from_inputs() {
        let v = serde_json::json!({"command": "cp src/a.rs /tmp/b.rs && cargo test --manifest-path=crates/x/Cargo.toml https://x.io/a.js"});
        assert_eq!(
            touched_paths(&v),
            ["src/a.rs", "/tmp/b.rs", "crates/x/Cargo.toml"]
        );
        assert_eq!(
            touched_paths(&serde_json::json!({"file_path": "/w/a.ts"})),
            ["/w/a.ts"]
        );
    }

    #[test]
    fn secrets_are_masked() {
        let m = mask_secrets(
            r#"{"command": "curl -H 'Authorization: Bearer abcdef123456' -d key=sk-ant-api03-ABCDEFGHIJKLMNOP && export GITHUB_TOKEN=ghp_1234567890abcdefghij && PASSWORD=hunter2hunter"}"#,
        );
        assert!(!m.contains("abcdef123456"), "{m}");
        assert!(!m.contains("ABCDEFGHIJKLMNOP"), "{m}");
        assert!(!m.contains("1234567890abcdefghij"), "{m}");
        assert!(!m.contains("hunter2hunter"), "{m}");
        assert!(m.contains("curl") && m.contains("Authorization"), "{m}");
        assert_eq!(
            mask_secrets("cargo test -p antfarm-core"),
            "cargo test -p antfarm-core"
        );
        let path = "/Users/Dev/projects/My-App/src/very/long/Path2/that/keeps/going/File.ts";
        assert_eq!(mask_secrets(path), path, "paths are not keys");
        assert!(
            !mask_secrets("token: aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY3zA5bC7dE9")
                .contains("aB3dE5fG7h")
        );
        assert!(
            !mask_secrets("x aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY3zA5bC7dE9fG").contains("hJ9kL1m")
        );
        let json =
            mask_secrets("{\n  \"api_key\": \"abcdefgh12345\",\n  \"path\": \"src/main.rs\"\n}");
        assert!(
            !json.contains("abcdefgh12345") && json.contains("src/main.rs"),
            "{json}"
        );
    }

    #[test]
    fn masking_handles_review_cases() {
        for (input, secret) in [
            ("PASSWORD=abcäde", "cäde"),
            ("Authorization: Bearer aéééééé", "ééééé"),
            ("{\"api_key\": \"日本語テスト値\"}", "テスト値"),
            (
                "psql postgres://app:s3cretpass@db.internal/app",
                "s3cretpass",
            ),
            ("curl -u admin:hunter2hunter https://x", "hunter2hunter"),
            (
                "curl -H 'Authorization: Basic dXNlcjpzdXBlcnNlY3JldA=='",
                "pzdXBlcnNlY3JldA",
            ),
            ("key=AIzaSyD-9tSrke72PouQMnMX-a7eZSW0jkFMBWY", "9tSrke72"),
            (
                "NPM=npm_1234567890abcdefghijklmnopqrstuvwxyz",
                "567890abcdef",
            ),
            ("STRIPE=sk_live_51HabcdefGHIJKLmnop", "HabcdefGHIJ"),
        ] {
            assert!(
                !mask_secrets(input).contains(secret),
                "{input} -> {}",
                mask_secrets(input)
            );
        }
        for plain in [
            "\"max_tokens\": 128000",
            "\"tokenizer\": \"cl100k_base\"",
            "\"keyboard\": \"layout-us\"",
            "--key-file=/etc/ssl/private/server.pem",
            "jest -t shouldReturn404WhenUserNotFoundAndRetryCountExceeded2",
            "https://github.com/2Toad/antfarm",
            "git -u origin main",
        ] {
            assert_eq!(mask_secrets(plain), plain);
        }
    }

    #[test]
    fn waterfall_collapses_runs() {
        let mut a = Agent::new(AgentId::from("x"), "root");
        for (i, (n, s, st)) in [
            ("Read", "src/a.rs", ResultStatus::Ok),
            ("Bash", "vitest --seed=1", ResultStatus::Ok),
            ("Bash", "vitest --seed=2", ResultStatus::Ok),
            ("Bash", "vitest --seed=3", ResultStatus::Error),
            ("Edit", "src/a.rs", ResultStatus::Pending),
        ]
        .into_iter()
        .enumerate()
        {
            a.push_activity(Activity::Call {
                call: ToolCallId::from(format!("c{i}")),
                name: n.into(),
                summary: s.into(),
                at: None,
                finished: None,
                status: st,
                input: None,
                why: None,
                background: None,
            });
        }
        let w = waterfall(&a);
        assert_eq!(
            w.iter()
                .map(|x| (x.first, x.count, x.tool.as_str()))
                .collect::<Vec<_>>(),
            [
                (1, 1, "Read"),
                (2, 2, "Bash"),
                (4, 1, "Bash"),
                (5, 1, "Edit")
            ]
        );
    }
}
