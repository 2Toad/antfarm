//! Pure status derivation for sessions and agents.
//!
//! Nothing here reads a clock or a file. The engine calls these after every change and
//! on every tick with the time it considers "now", so replay and live agree and the
//! rules can be table tested.

use chrono::Duration;
use indexmap::IndexMap;

use crate::model::{
    Agent, AgentId, AgentStatus, RegistryInfo, RegistryStatus, SessionStatus, Time, ToolCategory,
    WaitingKind,
};

/// A busy session with no transcript write for this long is `Still`.
pub const STILL_AFTER_SECS: i64 = 15 * 60;
/// Without a registry entry, a write within this many seconds means busy.
pub const REPLAY_BUSY_SECS: i64 = 60;
/// Longest chain of nested foreground spawns followed when finding the leaf.
const MAX_SPAWN_DEPTH: usize = 32;

/// The deepest agent reachable from the root by following open foreground spawn calls:
/// the one whose open calls say what the session is doing right now.
pub fn leaf_agent<'a>(agents: &'a IndexMap<AgentId, Agent>, root: &'a AgentId) -> &'a AgentId {
    let mut current = root;
    for _ in 0..MAX_SPAWN_DEPTH {
        let Some(agent) = agents.get(current) else {
            return current;
        };
        let child = agent
            .open
            .values()
            .filter(|c| c.category == ToolCategory::Spawn)
            .find_map(|call| {
                agents
                    .values()
                    .find(|a| a.spawned_by.as_ref() == Some(&call.id) && a.finished.is_none())
            });
        match child {
            Some(c) if &c.id != current => current = &c.id,
            _ => return current,
        }
    }
    current
}

fn older_than(now: Time, then: Option<Time>, secs: i64) -> bool {
    then.is_some_and(|t| now - t > Duration::seconds(secs))
}

/// Derives the session status. See the module docs of `engine` for the rules.
pub fn session_status(
    registry: Option<&RegistryInfo>,
    agents: &IndexMap<AgentId, Agent>,
    root: &AgentId,
    last_write: Option<Time>,
    now: Time,
) -> SessionStatus {
    if let Some(r) = registry
        && !r.alive
    {
        return SessionStatus::Exited;
    }
    let leaf = leaf_agent(agents, root);
    let open: Vec<_> = agents
        .get(leaf)
        .map(|a| a.open.values().collect())
        .unwrap_or_default();
    let question = open
        .iter()
        .any(|c| c.category == ToolCategory::Ask || c.name == "AskUserQuestion");
    let stale = older_than(now, last_write, STILL_AFTER_SECS);

    let Some(r) = registry else {
        // Replay, or the registry has not been seen yet: a recent write means busy.
        let busy = last_write.is_some_and(|w| now - w <= Duration::seconds(REPLAY_BUSY_SECS));
        return match (busy, open.is_empty()) {
            (true, false) => SessionStatus::Working,
            (true, true) => SessionStatus::Thinking,
            (false, _) if question => SessionStatus::Waiting(WaitingKind::Question),
            (false, _) => SessionStatus::Idle,
        };
    };

    match r.status {
        RegistryStatus::Busy => {
            if stale {
                SessionStatus::Still
            } else if open.is_empty() {
                SessionStatus::Thinking
            } else {
                SessionStatus::Working
            }
        }
        RegistryStatus::Idle | RegistryStatus::Waiting | RegistryStatus::Other(_) => {
            // A permission prompt always has its call open. Claude Code 2.1.318 writes an
            // AskUserQuestion call only once it is answered, so a wait with nothing open and
            // a label that isn't a permission prompt ("input needed") is a question.
            let unlogged_question = open.is_empty()
                && r.status == RegistryStatus::Waiting
                && r.waiting_for
                    .as_deref()
                    .is_some_and(|w| !w.contains("permission"));
            if question || unlogged_question {
                SessionStatus::Waiting(WaitingKind::Question)
            } else if !open.is_empty() || r.status == RegistryStatus::Waiting {
                SessionStatus::Waiting(WaitingKind::Permission)
            } else {
                SessionStatus::Idle
            }
        }
    }
}

/// Derives one agent's status. `leaf` says whether this agent is the session's leaf,
/// in which case it mirrors the session; `session` is the status just derived for it.
pub fn agent_status(agent: &Agent, leaf: bool, session: SessionStatus, now: Time) -> AgentStatus {
    if agent.finished.is_some() {
        return AgentStatus::Finished;
    }
    if session == SessionStatus::Exited {
        return AgentStatus::Exited;
    }
    // The root agent's staleness is the session's: an idle root at the prompt is not stuck.
    if !agent.is_root() && older_than(now, agent.last_write, STILL_AFTER_SECS) {
        return AgentStatus::Still;
    }
    if leaf {
        return match session {
            SessionStatus::Working => AgentStatus::Working,
            SessionStatus::Waiting(k) => AgentStatus::Waiting(k),
            SessionStatus::Still => AgentStatus::Still,
            SessionStatus::Exited => AgentStatus::Exited,
            SessionStatus::Thinking => AgentStatus::Thinking,
            SessionStatus::Idle => AgentStatus::Idle,
        };
    }
    if agent.open.is_empty() {
        AgentStatus::Thinking
    } else {
        AgentStatus::Working
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ResultStatus, ToolCall, ToolCallId};
    use chrono::TimeZone;
    use std::sync::Arc;

    fn t0() -> Time {
        chrono::Utc.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).unwrap()
    }

    fn call(agent: &AgentId, id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::from(id),
            agent: agent.clone(),
            name: name.into(),
            category: ToolCategory::of(name),
            input: Arc::new(serde_json::json!({})),
            summary: name.into(),
            started: Some(t0()),
            finished: None,
            status: ResultStatus::Pending,
            result_bytes: 0,
            parallel_group: None,
        }
    }

    fn registry(status: RegistryStatus, alive: bool) -> RegistryInfo {
        RegistryInfo {
            status,
            waiting_for: None,
            status_updated: None,
            alive,
        }
    }

    /// A root agent with the given open tool, if any.
    fn agents(open: Option<&str>) -> (IndexMap<AgentId, Agent>, AgentId) {
        let root = AgentId::from("root");
        let mut a = Agent::root(root.clone());
        a.last_write = Some(t0());
        if let Some(name) = open {
            let c = call(&root, "t1", name);
            a.open.insert(c.id.clone(), c);
        }
        let mut m = IndexMap::new();
        m.insert(root.clone(), a);
        (m, root)
    }

    #[test]
    fn table() {
        use RegistryStatus::*;
        use SessionStatus as S;
        use WaitingKind::*;
        // (registry, open tool, minutes since last write) -> status
        #[derive(Clone)]
        enum Reg {
            None,
            Live(RegistryStatus),
            Dead,
        }
        let fresh = 0;
        let old = 20;
        let cases: Vec<(Reg, Option<&str>, i64, S)> = vec![
            (Reg::None, None, fresh, S::Thinking),
            (Reg::None, Some("Bash"), fresh, S::Working),
            (Reg::None, Some("AskUserQuestion"), fresh, S::Working),
            (Reg::None, None, old, S::Idle),
            (Reg::None, Some("Bash"), old, S::Idle),
            (
                Reg::None,
                Some("AskUserQuestion"),
                old,
                S::Waiting(Question),
            ),
            (Reg::Live(Busy), None, fresh, S::Thinking),
            (Reg::Live(Busy), Some("Bash"), fresh, S::Working),
            (Reg::Live(Busy), Some("AskUserQuestion"), fresh, S::Working),
            (Reg::Live(Busy), None, old, S::Still),
            (Reg::Live(Busy), Some("Bash"), old, S::Still),
            (Reg::Live(Busy), Some("AskUserQuestion"), old, S::Still),
            (Reg::Live(Idle), None, fresh, S::Idle),
            (Reg::Live(Idle), Some("Bash"), fresh, S::Waiting(Permission)),
            (
                Reg::Live(Idle),
                Some("AskUserQuestion"),
                fresh,
                S::Waiting(Question),
            ),
            (Reg::Live(Idle), None, old, S::Idle),
            (Reg::Live(Idle), Some("Bash"), old, S::Waiting(Permission)),
            (
                Reg::Live(Idle),
                Some("AskUserQuestion"),
                old,
                S::Waiting(Question),
            ),
            (Reg::Live(Waiting), None, fresh, S::Waiting(Permission)),
            (
                Reg::Live(Waiting),
                Some("Bash"),
                fresh,
                S::Waiting(Permission),
            ),
            (
                Reg::Live(Waiting),
                Some("AskUserQuestion"),
                fresh,
                S::Waiting(Question),
            ),
            (Reg::Live(Waiting), None, old, S::Waiting(Permission)),
            (
                Reg::Live(Waiting),
                Some("Bash"),
                old,
                S::Waiting(Permission),
            ),
            (
                Reg::Live(Waiting),
                Some("AskUserQuestion"),
                old,
                S::Waiting(Question),
            ),
            (Reg::Dead, None, fresh, S::Exited),
            (Reg::Dead, Some("Bash"), fresh, S::Exited),
            (Reg::Dead, Some("AskUserQuestion"), fresh, S::Exited),
            (Reg::Dead, None, old, S::Exited),
            (Reg::Dead, Some("Bash"), old, S::Exited),
            (Reg::Dead, Some("AskUserQuestion"), old, S::Exited),
            (
                Reg::Live(Other("odd".into())),
                Some("Bash"),
                fresh,
                S::Waiting(Permission),
            ),
            (Reg::Live(Other("odd".into())), None, fresh, S::Idle),
        ];
        for (reg, open, minutes, want) in cases {
            let (agents, root) = agents(open);
            let r = match reg.clone() {
                Reg::None => None,
                Reg::Live(s) => Some(registry(s, true)),
                Reg::Dead => Some(registry(Busy, false)),
            };
            let now = t0() + Duration::minutes(minutes);
            let got = session_status(r.as_ref(), &agents, &root, Some(t0()), now);
            assert_eq!(
                got,
                want,
                "registry {:?} open {:?} age {}m",
                r.as_ref().map(|r| (r.status.clone(), r.alive)),
                open,
                minutes
            );
            // The root agent is the leaf here and mirrors the session, staleness included.
            let a = &agents[&root];
            let as_ = agent_status(a, true, got, now);
            let expect = match got {
                S::Exited => AgentStatus::Exited,
                S::Working => AgentStatus::Working,
                S::Waiting(k) => AgentStatus::Waiting(k),
                S::Still => AgentStatus::Still,
                S::Thinking => AgentStatus::Thinking,
                S::Idle => AgentStatus::Idle,
            };
            assert_eq!(as_, expect, "agent for {got:?} at {minutes}m");
        }
    }

    #[test]
    fn no_write_yet_is_never_stale() {
        let (mut agents, root) = agents(None);
        agents[&root].last_write = None;
        let r = registry(RegistryStatus::Busy, true);
        assert_eq!(
            session_status(Some(&r), &agents, &root, None, t0()),
            SessionStatus::Thinking
        );
        assert_eq!(
            session_status(None, &agents, &root, None, t0()),
            SessionStatus::Idle
        );
    }

    #[test]
    fn nested_foreground_spawn_uses_the_deepest_open_calls() {
        let root = AgentId::from("root");
        let mid = AgentId::from("mid");
        let leaf = AgentId::from("leaf");
        let mut r = Agent::root(root.clone());
        let spawn1 = call(&root, "sp1", "Agent");
        r.open.insert(spawn1.id.clone(), spawn1);
        let mut m = Agent::new(mid.clone(), "general-purpose");
        m.parent = Some(root.clone());
        m.spawned_by = Some(ToolCallId::from("sp1"));
        let spawn2 = call(&mid, "sp2", "Agent");
        m.open.insert(spawn2.id.clone(), spawn2);
        let mut l = Agent::new(leaf.clone(), "Explore");
        l.parent = Some(mid.clone());
        l.spawned_by = Some(ToolCallId::from("sp2"));
        let ask = call(&leaf, "q1", "AskUserQuestion");
        l.open.insert(ask.id.clone(), ask);
        for a in [&mut r, &mut m, &mut l] {
            a.last_write = Some(t0());
        }
        let mut agents = IndexMap::new();
        agents.insert(root.clone(), r);
        agents.insert(mid.clone(), m);
        agents.insert(leaf.clone(), l);

        assert_eq!(leaf_agent(&agents, &root), &leaf);
        let idle = registry(RegistryStatus::Idle, true);
        let s = session_status(Some(&idle), &agents, &root, Some(t0()), t0());
        assert_eq!(s, SessionStatus::Waiting(WaitingKind::Question));
        assert_eq!(
            agent_status(&agents[&root], false, s, t0()),
            AgentStatus::Working
        );
        assert_eq!(
            agent_status(&agents[&leaf], true, s, t0()),
            AgentStatus::Waiting(WaitingKind::Question)
        );

        // Once the leaf finishes, the chain stops at mid, whose only open call is the
        // spawn itself.
        agents[&leaf].finished = Some(t0());
        assert_eq!(leaf_agent(&agents, &root), &mid);
        let busy = registry(RegistryStatus::Busy, true);
        assert_eq!(
            session_status(Some(&busy), &agents, &root, Some(t0()), t0()),
            SessionStatus::Working
        );
        assert_eq!(
            agent_status(&agents[&leaf], false, SessionStatus::Working, t0()),
            AgentStatus::Finished
        );

        // A background spawn whose call already closed does not redirect the leaf.
        agents[&root].open.clear();
        assert_eq!(leaf_agent(&agents, &root), &root);
    }
    #[test]
    fn a_wait_with_nothing_open_follows_the_registry_label() {
        let (agents, root) = agents(None);
        let status = |waiting_for: Option<&str>| {
            let mut r = registry(RegistryStatus::Waiting, true);
            r.waiting_for = waiting_for.map(Into::into);
            session_status(Some(&r), &agents, &root, Some(t0()), t0())
        };
        assert_eq!(
            status(Some("input needed")),
            SessionStatus::Waiting(WaitingKind::Question)
        );
        assert_eq!(
            status(Some("permission prompt")),
            SessionStatus::Waiting(WaitingKind::Permission)
        );
        assert_eq!(
            status(None),
            SessionStatus::Waiting(WaitingKind::Permission)
        );
    }
}
