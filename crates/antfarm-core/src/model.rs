//! The generic model every source maps onto and every front end renders.
//!
//! Nothing here is specific to Claude Code. A source turns its files into these types;
//! the engine keeps them current; front ends read snapshots of them.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type Time = DateTime<Utc>;

/// How many activity entries an agent keeps. Older entries are dropped from the front.
pub const ACTIVITY_CAP: usize = 500;

/// How many of an agent's most recent calls keep their input in memory. Edit and Write
/// inputs hold whole files, so older ones are dropped.
pub const KEEP_INPUTS: usize = 50;

/// How many one-minute buckets an agent keeps for rates and sparklines.
pub const MINUTE_BUCKETS: usize = 10;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

string_id!(SessionId);
string_id!(AgentId);
string_id!(ToolCallId);

/// Every session antfarm knows about, keyed by session id in discovery order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub sessions: IndexMap<SessionId, Session>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub pid: Option<u32>,
    /// Short display name: registry name, then agent-name or custom title.
    pub name: Option<String>,
    /// Longer title, usually the AI-generated one.
    pub title: Option<String>,
    /// The first human prompt seen, shown as the session's task.
    pub task: Option<String>,
    pub last_prompt: Option<String>,
    pub project: PathBuf,
    pub branch: Option<String>,
    pub entrypoint: Option<String>,
    pub version: Option<String>,
    /// The app the session runs in, when it could be found from its parent processes.
    pub source: Option<SourceApp>,
    pub status: SessionStatus,
    pub status_since: Option<Time>,
    pub registry: Option<RegistryInfo>,
    pub started: Option<Time>,
    pub last_write: Option<Time>,
    pub cost: Cost,
    pub usage: Usage,
    pub context: ContextFill,
    pub root: AgentId,
    pub agents: IndexMap<AgentId, Agent>,
    pub workflows: Vec<WorkflowRun>,
    pub files_touched: Vec<FileTouch>,
    pub lines_added: Option<u64>,
    pub lines_removed: Option<u64>,
    /// Byte offset in the main transcript before which history was not read.
    pub truncated_before: Option<u64>,
    pub parse: ParseStats,
}

impl Session {
    pub fn new(id: SessionId, project: PathBuf) -> Self {
        let root = AgentId(id.0.clone());
        let mut agents = IndexMap::new();
        agents.insert(root.clone(), Agent::root(root.clone()));
        Self {
            id,
            pid: None,
            name: None,
            title: None,
            task: None,
            last_prompt: None,
            project,
            branch: None,
            entrypoint: None,
            version: None,
            source: None,
            status: SessionStatus::Idle,
            status_since: None,
            registry: None,
            started: None,
            last_write: None,
            cost: Cost::default(),
            usage: Usage::default(),
            context: ContextFill::default(),
            root,
            agents,
            workflows: Vec::new(),
            files_touched: Vec::new(),
            lines_added: None,
            lines_removed: None,
            truncated_before: None,
            parse: ParseStats::default(),
        }
    }

    pub fn root_agent(&self) -> &Agent {
        &self.agents[&self.root]
    }

    /// All open tool calls in the session, across agents.
    pub fn open_calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.agents.values().flat_map(|a| a.open.values())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Busy with a tool call open.
    Working,
    /// Busy with nothing open: the model is thinking or streaming text.
    Thinking,
    /// Blocked on the human.
    Waiting(WaitingKind),
    /// Idle with nothing open: done for now.
    Idle,
    /// Busy but nothing written for a long time: possibly hung.
    Still,
    /// The process is gone.
    Exited,
}

impl SessionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Thinking => "thinking",
            Self::Waiting(WaitingKind::Permission) => "waiting · permission",
            Self::Waiting(WaitingKind::Question) => "waiting · question",
            Self::Idle => "idle",
            Self::Still => "still",
            Self::Exited => "exited",
        }
    }

    pub fn needs_human(&self) -> bool {
        matches!(self, Self::Waiting(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitingKind {
    Permission,
    Question,
}

/// What the live registry says, kept raw so `doctor` can show it next to our derivation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistryInfo {
    pub status: RegistryStatus,
    pub waiting_for: Option<String>,
    pub status_updated: Option<Time>,
    pub alive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryStatus {
    Busy,
    Idle,
    Waiting,
    Other(String),
}

impl RegistryStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "busy" => Self::Busy,
            "idle" => Self::Idle,
            "waiting" => Self::Waiting,
            other => Self::Other(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub parent: Option<AgentId>,
    /// The tool call in the parent that spawned this agent.
    pub spawned_by: Option<ToolCallId>,
    /// `root` for the session's main agent, otherwise the sub-agent type.
    pub kind: String,
    pub description: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub depth: u8,
    pub background: bool,
    pub status: AgentStatus,
    pub started: Option<Time>,
    pub finished: Option<Time>,
    pub last_write: Option<Time>,
    pub usage: Usage,
    pub cost_usd: Option<f64>,
    pub context: ContextFill,
    pub calls: u64,
    /// The last [`MINUTE_BUCKETS`] minutes of calls and new tokens, by line time.
    pub minutes: VecDeque<MinuteBucket>,
    pub open: IndexMap<ToolCallId, ToolCall>,
    pub activity: VecDeque<Activity>,
    pub children: Vec<AgentId>,
}

impl Agent {
    pub fn new(id: AgentId, kind: impl Into<String>) -> Self {
        Self {
            id,
            parent: None,
            spawned_by: None,
            kind: kind.into(),
            description: None,
            model: None,
            effort: None,
            depth: 0,
            background: false,
            status: AgentStatus::Working,
            started: None,
            finished: None,
            last_write: None,
            usage: Usage::default(),
            cost_usd: None,
            context: ContextFill::default(),
            calls: 0,
            minutes: VecDeque::new(),
            open: IndexMap::new(),
            activity: VecDeque::new(),
            children: Vec::new(),
        }
    }

    pub fn root(id: AgentId) -> Self {
        Self::new(id, "root")
    }

    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// Adds to the one-minute bucket holding `at`, keeping the newest
    /// [`MINUTE_BUCKETS`] buckets. Lines can arrive out of time order.
    pub fn bump_minute(&mut self, at: Option<Time>, calls: u32, tokens: u64) {
        let Some(at) = at else { return };
        let start = at
            - chrono::Duration::seconds(at.timestamp().rem_euclid(60))
            - chrono::Duration::nanoseconds(at.timestamp_subsec_nanos() as i64);
        let i = self.minutes.partition_point(|b| b.start < start);
        if self.minutes.get(i).is_none_or(|b| b.start != start) {
            if self.minutes.len() == MINUTE_BUCKETS && i == 0 {
                return;
            }
            self.minutes.insert(
                i,
                MinuteBucket {
                    start,
                    calls: 0,
                    tokens: 0,
                },
            );
        }
        let i = self.minutes.partition_point(|b| b.start < start);
        self.minutes[i].calls += calls;
        self.minutes[i].tokens += tokens;
        while self.minutes.len() > MINUTE_BUCKETS {
            self.minutes.pop_front();
        }
    }

    pub fn push_activity(&mut self, a: Activity) {
        if self.activity.len() == ACTIVITY_CAP {
            self.activity.pop_front();
        }
        let is_call = matches!(a, Activity::Call { .. });
        self.activity.push_back(a);
        if is_call {
            // Older calls already dropped theirs, so only the one now past the limit can
            // still hold an input.
            let mut past = self.activity.iter_mut().rev().filter_map(|x| match x {
                Activity::Call { input, .. } => Some(input),
                _ => None,
            });
            if let Some(input) = past.nth(KEEP_INPUTS) {
                *input = None;
            }
        }
    }
}

/// Calls started and new tokens (input, cache writes, and output; not cache reads) in
/// one minute of line time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinuteBucket {
    pub start: Time,
    pub calls: u32,
    pub tokens: u64,
}

/// Where a session runs: the app found among its parent processes, and its tmux pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceApp {
    /// A display name that is also what `open -a` takes on macOS, such as
    /// `Visual Studio Code` or `iTerm`, or a process name such as `tmux` or `kitty`.
    pub app: String,
    /// `session:window.pane` when the session runs inside tmux.
    pub tmux_pane: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Working,
    Thinking,
    Waiting(WaitingKind),
    /// The root agent of an idle session.
    Idle,
    Finished,
    Still,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub agent: AgentId,
    pub name: String,
    pub category: ToolCategory,
    pub input: Arc<serde_json::Value>,
    /// One line, safe to show by default. Never the full input.
    pub summary: String,
    pub started: Option<Time>,
    pub finished: Option<Time>,
    pub status: ResultStatus,
    pub result_bytes: u64,
    /// The assistant message id; calls sharing it were issued in parallel.
    pub parallel_group: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    #[default]
    Pending,
    Ok,
    Error,
}

/// Coarse tool families, used for colors in the swimlanes and glyphs on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    Shell,
    Read,
    Edit,
    Search,
    Web,
    Spawn,
    Ask,
    Other,
}

impl ToolCategory {
    pub fn of(tool_name: &str) -> Self {
        match tool_name {
            "Bash" | "BashOutput" | "KillShell" | "Monitor" | "PowerShell" => Self::Shell,
            "Read" | "NotebookRead" | "LS" => Self::Read,
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => Self::Edit,
            "Grep" | "Glob" | "ToolSearch" => Self::Search,
            "WebFetch" | "WebSearch" => Self::Web,
            "Agent" | "Task" | "Workflow" => Self::Spawn,
            "AskUserQuestion" => Self::Ask,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Activity {
    /// A tool call. `finished` and `status` are filled in when its result arrives.
    Call {
        call: ToolCallId,
        name: String,
        summary: String,
        at: Option<Time>,
        #[serde(default)]
        finished: Option<Time>,
        #[serde(default)]
        status: ResultStatus,
        /// The call's input, kept in memory for the call screen and never serialized,
        /// since an Edit or Write input can hold a whole file.
        #[serde(skip)]
        input: Option<Arc<serde_json::Value>>,
    },
    Text {
        at: Option<Time>,
    },
    Prompt {
        at: Option<Time>,
        len: usize,
    },
    Compaction {
        at: Option<Time>,
        pre: Option<u64>,
        post: Option<u64>,
        trigger: Option<String>,
    },
    Spawn {
        at: Option<Time>,
        child: AgentId,
    },
}

/// Token counts summed over deduplicated assistant messages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// All cache writes; `cache_creation_1h` is the part written to the 1 hour cache.
    pub cache_creation: u64,
    pub cache_creation_1h: u64,
    pub thinking: u64,
    pub messages: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_creation += other.cache_creation;
        self.cache_creation_1h += other.cache_creation_1h;
        self.thinking += other.thinking;
        self.messages += other.messages;
    }

    /// Tokens the model saw as its prompt on this message.
    pub fn prompt_tokens(&self) -> u64 {
        self.input + self.cache_read + self.cache_creation
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// The last cost Claude Code itself reported, from a `cost-state` line.
    pub reported_usd: Option<f64>,
    /// Priced usage seen after the last report.
    pub delta_usd: f64,
    pub basis: CostBasis,
    pub usd_per_min: f64,
}

impl Cost {
    pub fn total(&self) -> f64 {
        self.reported_usd.unwrap_or(0.0) + self.delta_usd
    }

    pub fn is_estimate(&self) -> bool {
        self.basis != CostBasis::Reported
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    /// Anchored on a cost-state line plus priced usage after it.
    Reported,
    /// Only priced usage from the part of the transcript that was read.
    TailEstimate,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextFill {
    pub tokens: u64,
    pub window: Option<u64>,
    pub pct: Option<f32>,
}

impl ContextFill {
    pub fn set(&mut self, tokens: u64, window: Option<u64>) {
        self.tokens = tokens;
        self.window = window;
        self.pct = window
            .filter(|w| *w > 0)
            .map(|w| (tokens as f64 / w as f64 * 100.0) as f32);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTouch {
    pub path: String,
    pub agent: AgentId,
    pub tool: String,
    pub at: Option<Time>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub run_id: String,
    pub name: Option<String>,
    pub status: Option<String>,
    pub started: Option<Time>,
    pub duration_ms: Option<u64>,
    pub phases: Vec<Phase>,
    pub agents: Vec<WorkflowAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    pub index: u32,
    pub title: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowAgent {
    pub agent: AgentId,
    pub label: Option<String>,
    pub phase_index: Option<u32>,
    pub state: Option<String>,
}

/// Parser and pairing counters for one session, surfaced by `antfarm doctor`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseStats {
    pub lines: u64,
    pub bytes: u64,
    pub parsed: u64,
    pub errors: u64,
    pub first_error: Option<String>,
    pub oversized: u64,
    pub ignored_types: BTreeMap<String, u64>,
    pub orphan_results: u64,
    pub orphan_after_truncation: u64,
    pub versions: BTreeMap<String, u64>,
}

#[cfg(test)]
mod minute_tests {
    use super::*;

    fn t(s: &str) -> Option<Time> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn buckets_by_minute_out_of_order_and_capped() {
        let mut a = Agent::new(AgentId::from("a"), "x");
        a.bump_minute(t("2026-11-03T10:05:10Z"), 1, 100);
        a.bump_minute(t("2026-11-03T10:05:50Z"), 1, 50);
        a.bump_minute(t("2026-11-03T10:03:00Z"), 0, 7);
        assert_eq!(a.minutes.len(), 2);
        assert_eq!(a.minutes[0].tokens, 7);
        assert_eq!((a.minutes[1].calls, a.minutes[1].tokens), (2, 150));
        for m in 0..15 {
            a.bump_minute(t(&format!("2026-11-03T11:{m:02}:00Z")), 1, 1);
        }
        assert_eq!(a.minutes.len(), MINUTE_BUCKETS);
        assert_eq!(a.minutes[0].start, t("2026-11-03T11:05:00Z").unwrap());
        // Older than the window once it is full: dropped.
        a.bump_minute(t("2026-11-03T10:00:00Z"), 1, 1);
        assert_eq!(a.minutes[0].start, t("2026-11-03T11:05:00Z").unwrap());
    }
}
