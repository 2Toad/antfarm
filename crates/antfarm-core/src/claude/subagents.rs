//! Builds the agent tree from the three places Claude Code records a spawn.
//!
//! - The parent's `Agent` tool call. A background spawn's result arrives at once with
//!   `toolUseResult: {isAsync: true, agentId}`; a foreground spawn's result arrives when
//!   the sub-agent returns.
//! - The sub-agent's `agent-<id>.meta.json`: type, description, depth, the spawning
//!   call (`toolUseId`), the parent agent for nested spawns, and `finished`.
//! - A `<task-notification>` user message in the parent naming the agent as `task-id`
//!   with a terminal `status`.
//!
//! Any of these can arrive first, or be missing because it fell before the tail. Each
//! one creates the agent if needed and fills in what it knows.

use chrono::{DateTime, Utc};

use super::files::SubagentMeta;
use super::line::ToolUseResult;
use super::reduce::SessionSide;
use crate::engine::Emitter;
use crate::event::Event;
use crate::model::{Activity, Agent, AgentId, Session, Time, ToolCall, ToolCallId};

/// Tool names that spawn a sub-agent.
pub const SPAWN_TOOLS: &[&str] = &["Agent", "Task"];

/// `<status>` values that mean a background task is over.
const TERMINAL: &[&str] = &[
    "completed",
    "failed",
    "killed",
    "stopped",
    "cancelled",
    "error",
];

/// Creates `id` under `parent` if it does not exist, emitting `AgentSpawned`. An agent
/// that exists under a different parent is moved. Returns true when it was created.
pub(crate) fn ensure_agent(
    session: &mut Session,
    id: &AgentId,
    parent: &AgentId,
    at: Option<Time>,
    em: &mut Emitter,
) -> bool {
    if id == parent || *id == session.root {
        return false;
    }
    if !session.agents.contains_key(parent) {
        let root = session.root.clone();
        ensure_agent(session, parent, &root, at, em);
    }
    if session.agents.contains_key(id) {
        reparent(session, id, parent);
        return false;
    }
    let depth = session.agents[parent].depth.saturating_add(1);
    let mut a = Agent::new(id.clone(), "unknown");
    a.parent = Some(parent.clone());
    a.depth = depth;
    // An agent known only from its spawn is dated by it, so it can go still if nothing
    // else about it ever shows up.
    a.started = at;
    a.last_write = at;
    session.agents.insert(id.clone(), a);
    session.agents[parent].children.push(id.clone());
    em.emit(
        &session.id,
        at,
        Event::AgentSpawned {
            agent: id.clone(),
            parent: Some(parent.clone()),
            kind: "unknown".into(),
            description: None,
        },
    );
    true
}

fn reparent(session: &mut Session, id: &AgentId, parent: &AgentId) {
    let old = session.agents[id].parent.clone();
    if old.as_ref() == Some(parent) || is_ancestor(session, id, parent) {
        return;
    }
    if let Some(old) = old.and_then(|o| session.agents.get_mut(&o)) {
        old.children.retain(|c| c != id);
    }
    session.agents[parent].children.push(id.clone());
    let depth = session.agents[parent].depth.saturating_add(1);
    let a = &mut session.agents[id];
    a.parent = Some(parent.clone());
    if a.kind == "unknown" {
        a.depth = depth;
    }
}

/// True when `a` is `b` or one of `b`'s ancestors, which would make a cycle.
fn is_ancestor(session: &Session, a: &AgentId, b: &AgentId) -> bool {
    let mut cur = Some(b.clone());
    while let Some(c) = cur {
        if &c == a {
            return true;
        }
        cur = session.agents.get(&c).and_then(|x| x.parent.clone());
    }
    false
}

/// Marks an agent finished once and emits `AgentFinished`.
pub(crate) fn finish(session: &mut Session, id: &AgentId, at: Option<Time>, em: &mut Emitter) {
    let Some(a) = session.agents.get_mut(id) else {
        return;
    };
    if a.finished.is_some() || a.parent.is_none() {
        return;
    }
    a.finished = at.or(a.last_write).or(Some(Time::UNIX_EPOCH));
    em.emit(&session.id, at, Event::AgentFinished { agent: id.clone() });
}

/// Records which agent issued a spawn call, so a later result or meta can find the parent.
pub(crate) fn spawn_opened(side: &mut SessionSide, call: &ToolCallId, caller: &AgentId) {
    side.spawn_caller.insert(call.clone(), caller.clone());
}

/// Links the child named by a spawn call's result, or finishes it for a foreground
/// spawn that just returned.
pub(crate) fn spawn_closed(
    session: &mut Session,
    side: &mut SessionSide,
    call: &ToolCall,
    caller: &AgentId,
    result: Option<&ToolUseResult>,
    at: Option<Time>,
    em: &mut Emitter,
) {
    match result {
        Some(r) if r.is_async => {
            if let Some(child) = r.agent_id.as_deref().filter(|s| !s.is_empty()) {
                let child = AgentId::from(child);
                link(session, side, &child, &call.id, caller, at, em);
                let a = &mut session.agents[&child];
                a.background = true;
                if a.description.is_none() {
                    a.description = Some(call.summary.clone());
                }
            }
        }
        _ => {
            if let Some(child) = side.spawned.get(&call.id).cloned() {
                finish(session, &child, at, em);
            }
        }
    }
}

fn link(
    session: &mut Session,
    side: &mut SessionSide,
    child: &AgentId,
    call: &ToolCallId,
    parent: &AgentId,
    at: Option<Time>,
    em: &mut Emitter,
) {
    let created = ensure_agent(session, child, parent, at, em);
    let first_link = side.spawned.insert(call.clone(), child.clone()).is_none();
    session.agents[child].spawned_by = Some(call.clone());
    if (created || first_link)
        && let Some(p) = session.agents.get_mut(parent)
    {
        p.push_activity(Activity::Spawn {
            at,
            child: child.clone(),
        });
    }
}

/// Applies a sub-agent's meta file.
pub(crate) fn apply_meta(
    session: &mut Session,
    side: &mut SessionSide,
    id: &AgentId,
    meta: &SubagentMeta,
    transcript_mtime: Option<Time>,
    em: &mut Emitter,
) {
    let call = meta.tool_use_id.as_deref().map(ToolCallId::from);
    let parent = meta
        .parent_agent_id
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(AgentId::from)
        .or_else(|| {
            call.as_ref()
                .and_then(|c| side.spawn_caller.get(c).cloned())
        })
        .or_else(|| session.agents.get(id).and_then(|a| a.parent.clone()))
        .unwrap_or_else(|| session.root.clone());
    let at = transcript_mtime;
    match &call {
        Some(c) => link(session, side, id, c, &parent, at, em),
        None => {
            ensure_agent(session, id, &parent, at, em);
        }
    }
    let a = &mut session.agents[id];
    if let Some(t) = &meta.agent_type {
        a.kind = t.clone();
    }
    if meta.description.is_some() {
        a.description = meta.description.clone();
    }
    if a.model.is_none() {
        a.model = meta.model.clone();
    }
    if let Some(d) = meta.spawn_depth {
        a.depth = d;
    }
    if let Some(shape) = meta.request_shape.as_deref() {
        a.background = shape == "background";
    }
    if let Some(t) = transcript_mtime {
        a.last_write = Some(a.last_write.map_or(t, |x| x.max(t)));
        if a.started.is_none() {
            a.started = Some(t);
        }
    }
    if meta.finished == Some(true) {
        let when = a.last_write;
        finish(session, id, when, em);
    }
}

/// Finishes every agent named by a terminal `<task-notification>` in `text`.
pub(crate) fn task_notifications(
    session: &mut Session,
    text: &str,
    at: Option<Time>,
    em: &mut Emitter,
) {
    for (task, status) in parse_task_notifications(text) {
        let id = AgentId::from(task);
        if TERMINAL.contains(&status) && session.agents.contains_key(&id) {
            finish(session, &id, at, em);
        }
    }
}

/// Pulls `(task-id, status)` pairs out of `<task-notification>` blocks.
pub fn parse_task_notifications(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<task-notification>") {
        let body = &rest[start + "<task-notification>".len()..];
        let end = body.find("</task-notification>").unwrap_or(body.len());
        let block = &body[..end];
        if let (Some(id), Some(status)) = (tag(block, "task-id"), tag(block, "status")) {
            out.push((id.trim(), status.trim()));
        }
        rest = &body[end..];
    }
    out
}

fn tag<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let s = block.find(&open)? + open.len();
    let e = block[s..].find(&close)? + s;
    Some(&block[s..e])
}

/// Epoch milliseconds to a time, ignoring nonsense values.
pub(crate) fn from_ms(ms: i64) -> Option<Time> {
    DateTime::<Utc>::from_timestamp_millis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::parse::{ParsedLine, parse_line};
    use crate::engine::Engine;
    use crate::input::{AgentRef, Input};
    use crate::model::{AgentStatus, SessionId};

    fn sid() -> SessionId {
        SessionId::from("s")
    }

    fn lines(engine: &mut Engine, agent: AgentRef, json: &[&str]) -> Vec<Event> {
        let lines = json
            .iter()
            .map(|j| ParsedLine {
                offset: 0,
                len: j.len() as u64 + 1,
                parsed: parse_line(j.as_bytes(), 0).unwrap(),
            })
            .collect();
        engine
            .apply(Input::Lines {
                session: sid(),
                agent,
                lines,
                errors: vec![],
                end_offset: 0,
                mtime: None,
            })
            .into_iter()
            .map(|e| e.event)
            .collect()
    }

    fn meta(engine: &mut Engine, id: &str, json: &str) -> Vec<Event> {
        engine
            .apply(Input::SubagentMeta {
                session: sid(),
                agent: AgentId::from(id),
                meta: serde_json::from_str(json).unwrap(),
                transcript_mtime: None,
            })
            .into_iter()
            .map(|e| e.event)
            .collect()
    }

    const SPAWN: &str = r#"{"type":"assistant","timestamp":"2026-11-02T10:00:00Z","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_spawn","name":"Agent","input":{"description":"audit auth","prompt":"x"}}]}}"#;
    const ASYNC_RESULT: &str = r#"{"type":"user","timestamp":"2026-11-02T10:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_spawn","content":"launched"}]},"toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a1"}}"#;

    #[test]
    fn background_spawn_links_then_task_notification_finishes() {
        let mut e = Engine::new();
        lines(&mut e, AgentRef::Root, &[SPAWN, ASYNC_RESULT]);
        let s = &e.machine().sessions["s"];
        let a = &s.agents["a1"];
        assert_eq!(a.parent.as_ref().map(|p| p.as_str()), Some("s"));
        assert!(a.background);
        assert_eq!(a.description.as_deref(), Some("audit auth"));
        assert_eq!(
            a.spawned_by.as_ref().map(|c| c.as_str()),
            Some("toolu_spawn")
        );
        assert!(
            s.root_agent().open.is_empty(),
            "the async result closes the spawn call"
        );
        assert!(a.finished.is_none());

        let note = r#"{"type":"user","timestamp":"2026-11-02T10:05:00Z","message":{"content":"<task-notification>\n<task-id>a1</task-id>\n<tool-use-id>toolu_spawn</tool-use-id>\n<status>completed</status>\n<summary>done</summary>\n</task-notification>"}}"#;
        let ev = lines(&mut e, AgentRef::Root, &[note]);
        assert!(
            ev.iter()
                .any(|x| matches!(x, Event::AgentFinished { agent } if agent.as_str() == "a1"))
        );
        let s = &e.machine().sessions["s"];
        assert_eq!(s.agents["a1"].status, AgentStatus::Finished);
        assert!(s.task.is_none(), "a notification is not the human's task");
    }

    #[test]
    fn meta_before_spawn_and_result_ends_up_linked_once() {
        let mut e = Engine::new();
        let ev = meta(
            &mut e,
            "a1",
            r#"{"agentType":"Explore","description":"look around","toolUseId":"toolu_spawn","spawnDepth":1,"requestShape":"background","model":"sonnet"}"#,
        );
        assert_eq!(
            ev.iter()
                .filter(|x| matches!(x, Event::AgentSpawned { .. }))
                .count(),
            1
        );
        lines(&mut e, AgentRef::Root, &[SPAWN, ASYNC_RESULT]);
        let s = &e.machine().sessions["s"];
        assert_eq!(s.agents.len(), 2);
        assert_eq!(s.root_agent().children.len(), 1);
        let a = &s.agents["a1"];
        assert_eq!(a.kind, "Explore");
        assert_eq!(
            a.description.as_deref(),
            Some("look around"),
            "meta description wins"
        );
        assert_eq!(a.model.as_deref(), Some("sonnet"));
        let spawns = s
            .root_agent()
            .activity
            .iter()
            .filter(|x| matches!(x, Activity::Spawn { .. }))
            .count();
        assert_eq!(spawns, 1);
    }

    #[test]
    fn nested_foreground_spawn_and_placeholder_reparent() {
        let mut e = Engine::new();
        // The child's transcript shows up before anything names its parent.
        lines(
            &mut e,
            AgentRef::Sub(AgentId::from("b2")),
            &[
                r#"{"type":"user","timestamp":"2026-11-02T10:01:00Z","isSidechain":true,"message":{"content":"inner task"}}"#,
            ],
        );
        assert_eq!(
            e.machine().sessions["s"].agents["b2"]
                .parent
                .as_ref()
                .map(|p| p.as_str()),
            Some("s")
        );
        // The depth-1 agent issues a foreground spawn.
        lines(
            &mut e,
            AgentRef::Sub(AgentId::from("a1")),
            &[
                r#"{"type":"assistant","timestamp":"2026-11-02T10:00:50Z","message":{"id":"m2","content":[{"type":"tool_use","id":"toolu_fg","name":"Agent","input":{"description":"dig"}}]}}"#,
            ],
        );
        meta(
            &mut e,
            "b2",
            r#"{"agentType":"general-purpose","toolUseId":"toolu_fg","parentAgentId":"a1","spawnDepth":2,"requestShape":"foreground"}"#,
        );
        let s = &e.machine().sessions["s"];
        assert_eq!(
            s.agents["b2"].parent.as_ref().map(|p| p.as_str()),
            Some("a1")
        );
        assert_eq!(s.agents["b2"].depth, 2);
        assert!(!s.agents["b2"].background);
        assert_eq!(s.root_agent().children, [AgentId::from("a1")]);
        assert_eq!(s.agents["a1"].children, [AgentId::from("b2")]);
        // The foreground call returns: the child is finished.
        let ev = lines(
            &mut e,
            AgentRef::Sub(AgentId::from("a1")),
            &[
                r#"{"type":"user","timestamp":"2026-11-02T10:03:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_fg","content":"report"}]}}"#,
            ],
        );
        assert!(
            ev.iter()
                .any(|x| matches!(x, Event::AgentFinished { agent } if agent.as_str() == "b2"))
        );
    }

    #[test]
    fn meta_finished_marks_done_and_unknown_task_ids_are_ignored() {
        let mut e = Engine::new();
        lines(
            &mut e,
            AgentRef::Root,
            &[
                r#"{"type":"user","timestamp":"2026-11-02T10:00:00Z","message":{"content":"<task-notification><task-id>bash123</task-id><status>completed</status></task-notification>"}}"#,
            ],
        );
        assert_eq!(e.machine().sessions["s"].agents.len(), 1);
        meta(&mut e, "a9", r#"{"agentType":"Plan","finished":true}"#);
        assert!(e.machine().sessions["s"].agents["a9"].finished.is_some());
    }

    #[test]
    fn parses_several_notifications() {
        let t = "x <task-notification><task-id> a </task-id><status>completed</status></task-notification> y <task-notification><status>failed</status><task-id>b</task-id></task-notification><task-notification><task-id>c</task-id>";
        assert_eq!(
            parse_task_notifications(t),
            [("a", "completed"), ("b", "failed")]
        );
    }
}
