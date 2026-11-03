//! What sources feed the engine. Sources do all I/O; the engine only applies inputs.

use std::path::PathBuf;

use crate::claude::files::{RegistryEntry, SubagentMeta, WorkflowFile};
use crate::claude::parse::{ParseError, ParsedLine};
use crate::model::{AgentId, SessionId, Time};

/// Which transcript a batch of lines came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentRef {
    Root,
    Sub(AgentId),
}

#[derive(Debug, Clone)]
pub enum Input {
    Registry {
        entry: RegistryEntry,
        alive: bool,
    },
    RegistryGone {
        pid: u32,
    },
    /// A transcript was opened. `truncated_before` is set when history before it was skipped.
    Attached {
        session: SessionId,
        agent: AgentRef,
        path: PathBuf,
        truncated_before: Option<u64>,
    },
    Lines {
        session: SessionId,
        agent: AgentRef,
        lines: Vec<ParsedLine>,
        errors: Vec<ParseError>,
        end_offset: u64,
        mtime: Option<Time>,
    },
    FileTruncated {
        session: SessionId,
        agent: AgentRef,
    },
    SubagentMeta {
        session: SessionId,
        agent: AgentId,
        meta: SubagentMeta,
        transcript_mtime: Option<Time>,
    },
    Workflow {
        session: SessionId,
        file: WorkflowFile,
    },
    /// The last cost-state found by scanning backward from the end of the main
    /// transcript. Send it before that transcript's lines: usage already applied is not
    /// re-attributed. `at` is the line's timestamp when it has one; sub-agent usage
    /// before it is then excluded from the delta.
    CostAnchor {
        session: SessionId,
        usd: f64,
        offset: u64,
        at: Option<Time>,
    },
    Tick {
        now: Time,
    },
}
