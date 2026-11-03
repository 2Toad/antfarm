//! antfarm-core builds a live model of the Claude Code sessions on a machine from the
//! files Claude Code already writes under `~/.claude`.

pub mod claude;
pub mod event;
pub mod input;
pub mod model;
pub mod snapshot;

pub use event::{Event, EventEnvelope};
pub use model::*;
pub use snapshot::Snapshot;

use std::path::PathBuf;

/// Where Claude Code keeps its state. Defaults to `$HOME/.claude`, overridable with
/// `ANTFARM_CLAUDE_HOME` for tests and unusual setups.
#[derive(Debug, Clone)]
pub struct Paths {
    pub claude_home: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let claude_home = std::env::var_os("ANTFARM_CLAUDE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
                home.join(".claude")
            });
        Self { claude_home }
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.claude_home.join("sessions")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.claude_home.join("projects")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn snapshot_round_trips_through_json() {
        let mut session = Session::new(SessionId::from("s1"), "/tmp/p".into());
        session.status = SessionStatus::Waiting(WaitingKind::Question);
        let root = session.root.clone();
        let call = ToolCall {
            id: ToolCallId::from("toolu_1"),
            agent: root.clone(),
            name: "AskUserQuestion".into(),
            category: ToolCategory::of("AskUserQuestion"),
            input: Arc::new(serde_json::json!({"questions": []})),
            summary: "which palette?".into(),
            started: None,
            finished: None,
            status: ResultStatus::Pending,
            result_bytes: 0,
            parallel_group: Some("msg_1".into()),
        };
        let agent = session.agents.get_mut(&root).unwrap();
        agent.open.insert(call.id.clone(), call);
        agent.push_activity(Activity::Prompt { at: None, len: 12 });
        session.context.set(50_000, Some(200_000));
        let mut machine = Machine::default();
        machine.sessions.insert(session.id.clone(), session);
        let snap = Snapshot { seq: 7, at: chrono::Utc::now(), machine };

        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
        assert_eq!(back.machine.sessions["s1"].context.pct, Some(25.0));
    }
}
