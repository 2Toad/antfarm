//! What sources feed the engine. Sources do all I/O; the engine only applies inputs.

use std::path::PathBuf;

use crate::claude::files::{RegistryEntry, SubagentMeta, WorkflowFile};
use crate::claude::parse::{ParseError, ParsedLine};
use crate::model::{AgentId, CallRecord, FileTouch, SessionId, Time, ToolCallId};

/// Which transcript a batch of lines came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentRef {
    Root,
    Sub(AgentId),
}

/// A `Skill` call whose result named the sub-agent it forked.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkLink {
    pub call: ToolCallId,
    pub agent: AgentId,
    /// When the result arrived: when a foreground fork finished, or when a background
    /// one was sent off.
    pub at: Option<Time>,
    pub background: bool,
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
    /// A transcript was opened. `truncated_before` is set when history before it was
    /// skipped. `backfill` names the job that will read that history, when one will.
    Attached {
        session: SessionId,
        agent: AgentRef,
        path: PathBuf,
        truncated_before: Option<u64>,
        backfill: Option<u64>,
    },
    /// A tracked transcript moved, as when its session enters a git worktree.
    Relocated {
        session: SessionId,
        agent: AgentRef,
        path: PathBuf,
    },
    /// Older calls read backward from a transcript's tail boundary. Each batch is older
    /// than every batch before it from the same job, and holds `calls` in file order.
    /// Calls still pending had no result after them before the boundary.
    Backfill {
        session: SessionId,
        agent: AgentRef,
        job: u64,
        calls: Vec<CallRecord>,
        /// Files edited in this batch, one entry per path with its line counts summed.
        files: Vec<FileTouch>,
        /// Forked skills that returned in this batch.
        forks: Vec<ForkLink>,
        /// Bytes before the boundary read so far, and the boundary itself.
        read: u64,
        total: u64,
        done: bool,
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
    /// The repository and working tree the session's directory belongs to.
    SessionRepo {
        session: SessionId,
        repo: PathBuf,
        worktree: PathBuf,
    },
    /// The app a session runs in, found from its parent processes.
    SessionSource {
        session: SessionId,
        source: crate::model::SourceApp,
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
