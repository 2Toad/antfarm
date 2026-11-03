//! Typed transitions. Snapshots are the truth; events are for logs and animation.

use serde::{Deserialize, Serialize};

use crate::model::{AgentId, ResultStatus, SessionId, SessionStatus, Time, ToolCallId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub seq: u64,
    pub at: Time,
    pub session: Option<SessionId>,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SessionDiscovered {
        name: Option<String>,
    },
    SessionStatusChanged {
        from: SessionStatus,
        to: SessionStatus,
    },
    SessionExited,
    AgentSpawned {
        agent: AgentId,
        parent: Option<AgentId>,
        kind: String,
        description: Option<String>,
    },
    AgentFinished {
        agent: AgentId,
    },
    ToolCallStarted {
        agent: AgentId,
        call: ToolCallId,
        name: String,
        summary: String,
    },
    ToolCallFinished {
        agent: AgentId,
        call: ToolCallId,
        status: ResultStatus,
    },
    CostUpdated {
        usd: f64,
        estimate: bool,
    },
    Compacted {
        agent: AgentId,
        pre: Option<u64>,
        post: Option<u64>,
        trigger: Option<String>,
    },
    FileTruncated {
        agent: AgentId,
    },
    ParseError {
        agent: AgentId,
        offset: u64,
        message: String,
    },
    /// The receiver fell behind and missed events; re-read the snapshot.
    Resync,
}
