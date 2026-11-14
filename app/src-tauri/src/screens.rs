//! What each screen of the app shows, built from a snapshot by `antfarm-view`. The
//! webview receives exactly these structs as JSON; nothing else crosses the bridge.

use std::collections::HashMap;

use antfarm_core::{AgentId, SessionId, Snapshot, Time, ToolCallId, WaitingKind};
use antfarm_view::lanes::{self, LaneIntervals, Window};
use antfarm_view::project::{self, ProjectSummary, ProjectView};
use antfarm_view::{
    self as view, AgentLine, CallDetail, CallLine, Class, Meters, NeedsYou, Prefs, Row, Waterfall,
};
use serde::{Deserialize, Serialize};

/// Where the webview is, sent by `set_route`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "screen", rename_all = "snake_case")]
pub enum Route {
    Machine,
    Project {
        key: String,
    },
    Session {
        id: SessionId,
    },
    /// With `end` unset the window follows now and is `span` seconds wide; otherwise it
    /// is fixed at `start` to `end`. `width` is the drawing width in pixels.
    Lanes {
        id: SessionId,
        start: Option<Time>,
        end: Option<Time>,
        playhead: Option<Time>,
        #[serde(default)]
        span: Option<i64>,
        #[serde(default)]
        width: Option<u32>,
    },
    Agent {
        session: SessionId,
        agent: AgentId,
    },
    Call {
        session: SessionId,
        agent: AgentId,
        call: ToolCallId,
    },
}

/// Per-session notification choices, set from the session screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Notify {
    pub waiting: bool,
    pub finished: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            waiting: true,
            finished: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewEvent {
    pub seq: u64,
    pub now: Time,
    pub header: Header,
    pub body: Body,
    /// The tutorial's sample sessions, not the person's own.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sample: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Header {
    pub host: String,
    pub meters: Meters,
    pub needs_you: Vec<NeedsYou>,
    pub projects: Vec<ProjectSummary>,
    /// Every live session, for the jump palette and the sidebar.
    pub sessions: Vec<SessionItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionItem {
    pub id: SessionId,
    pub name: String,
    pub project: String,
    pub project_key: String,
    pub class: Class,
    pub state: String,
    pub tool: String,
    pub source: String,
    pub for_secs: Option<i64>,
    pub muted: bool,
    pub agents: Vec<AgentLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "screen", rename_all = "snake_case")]
pub enum Body {
    Machine(MachineBody),
    Project(Box<ProjectView>),
    Session(Box<SessionBody>),
    Lanes(Box<LanesBody>),
    Agent(Box<AgentBody>),
    Call(Box<CallBody>),
    Gone { what: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct MachineBody {
    pub rows: Vec<Row>,
    pub galleries: Vec<Gallery>,
    pub feed: Vec<FeedItem>,
    /// Calls per minute over the last ten minutes, oldest first.
    pub histogram: [u32; 10],
    pub calls_per_min: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gallery {
    pub key: String,
    pub name: String,
    pub waiting: usize,
    pub working: usize,
    pub chambers: Vec<Chamber>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chamber {
    pub id: SessionId,
    pub name: String,
    pub class: Class,
    pub state: String,
    pub waiting: Option<WaitingKind>,
    pub for_secs: Option<i64>,
    pub tool: String,
    pub target: String,
    pub source: String,
    pub cost: String,
    pub agents: Vec<AgentLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedItem {
    pub session: SessionId,
    pub session_name: String,
    #[serde(flatten)]
    pub call: CallLine,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionBody {
    pub id: SessionId,
    pub name: String,
    pub project: String,
    pub project_key: String,
    pub branch: Option<String>,
    pub tree: String,
    pub source: String,
    pub pid: Option<u32>,
    pub class: Class,
    pub status: String,
    pub for_secs: Option<i64>,
    pub uptime_secs: Option<i64>,
    pub active_secs: Option<i64>,
    /// True once the root agent's kept activity is full, so `active_secs`
    /// covers only its most recent calls.
    pub active_partial: bool,
    pub last_call_secs: Option<i64>,
    pub task: Option<String>,
    pub cost: String,
    pub usd_per_min: f64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub context_pct: Option<f32>,
    pub calls: u64,
    pub waiting: Option<view::Waiting>,
    pub agents: Vec<AgentLine>,
    pub timeline_start: Option<Time>,
    pub lanes: Vec<view::Lane>,
    pub call_lines: Vec<CallLine>,
    pub files: Vec<FileItem>,
    pub notify: Notify,
    pub muted: bool,
    pub snoozed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileItem {
    pub path: String,
    pub added: u64,
    pub removed: u64,
    pub tool: String,
    pub at: Option<Time>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanesBody {
    pub id: SessionId,
    pub name: String,
    pub start: Time,
    pub end: Time,
    pub playhead: Time,
    pub following: bool,
    pub history_start: Option<Time>,
    /// Backfill's progress, or that older history isn't loaded; `None` when complete.
    pub history_note: Option<String>,
    pub lanes: Vec<LaneIntervals>,
    /// Lanes left out past the drawing limit.
    pub hidden_lanes: usize,
    pub agents_at: Vec<lanes::AgentAt>,
    pub calls_at: Vec<CallLine>,
    pub waiting: Option<view::Waiting>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentBody {
    pub session: SessionId,
    pub session_name: String,
    pub id: AgentId,
    /// False while an old sub-agent's transcript is still being read.
    pub read: bool,
    pub name: String,
    pub kind: String,
    pub class: Class,
    pub state: String,
    pub depth: u8,
    pub background: bool,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub cost: Option<f64>,
    pub context_pct: Option<f32>,
    pub elapsed_secs: Option<i64>,
    pub brief: Option<String>,
    pub parent: Option<String>,
    pub calls: Vec<Waterfall>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallBody {
    pub session: SessionId,
    pub session_name: String,
    pub agent: AgentId,
    pub agent_name: String,
    pub call: ToolCallId,
    pub detail: CallDetail,
    /// The newest part of a background command's output, when it can be read.
    pub output_tail: Option<String>,
    pub last_completed: Option<CallLine>,
}

fn secs(t: Option<Time>, now: Time) -> Option<i64> {
    t.map(|t| (now - t).num_seconds().max(0))
}

pub fn header(snap: &Snapshot, prefs: &Prefs, host: &str, now: Time) -> Header {
    let sessions = snap
        .machine
        .sessions
        .values()
        .filter(|s| s.status != antfarm_core::SessionStatus::Exited)
        .map(|s| {
            let (tool, _) = view::session_current(s);
            SessionItem {
                id: s.id.clone(),
                name: view::session_name(s),
                project: project_name(s),
                project_key: project::repo_key(s).display().to_string(),
                class: Class::of_session(s.status),
                state: s.status.label().to_owned(),
                tool,
                source: view::source_label(s),
                for_secs: secs(s.status_since, now),
                muted: prefs.muted.contains(&s.id),
                agents: view::agent_lines(s, now)
                    .into_iter()
                    .filter(|a| a.depth > 0 && matches!(a.class, Class::Working | Class::Waiting))
                    .collect(),
            }
        })
        .collect();
    Header {
        host: host.to_owned(),
        meters: view::meters(snap, now),
        needs_you: view::needs_you(snap, prefs, now),
        projects: project::projects(snap),
        sessions,
    }
}

fn project_name(s: &antfarm_core::Session) -> String {
    project::repo_key(s)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| antfarm_view::fmt::project_name(s))
}

pub fn body(
    snap: &Snapshot,
    route: &Route,
    prefs: &Prefs,
    notify: &HashMap<SessionId, Notify>,
    show_exited: bool,
    now: Time,
) -> Body {
    let gone = |what: &str| Body::Gone { what: what.into() };
    match route {
        Route::Machine => Body::Machine(machine(snap, prefs, show_exited, now)),
        Route::Project { key } => project::project_view(snap, key, now)
            .map(|v| Body::Project(Box::new(v)))
            .unwrap_or_else(|| gone("project")),
        Route::Session { id } => match snap.machine.sessions.get(id) {
            Some(s) => Body::Session(Box::new(session(
                s,
                prefs,
                notify.get(id).copied().unwrap_or_default(),
                now,
            ))),
            None => gone("session"),
        },
        Route::Lanes {
            id,
            start,
            end,
            playhead,
            span,
            width,
        } => match snap.machine.sessions.get(id) {
            Some(s) => Body::Lanes(Box::new(lanes_body(
                s,
                *start,
                *end,
                *playhead,
                *span,
                width.unwrap_or(1200),
                now,
            ))),
            None => gone("session"),
        },
        Route::Agent { session, agent } => match snap
            .machine
            .sessions
            .get(session)
            .and_then(|s| s.agents.get(agent).map(|a| (s, a)))
        {
            Some((s, a)) => Body::Agent(Box::new(agent_body(s, a, now))),
            None => gone("agent"),
        },
        Route::Call {
            session,
            agent,
            call,
        } => {
            let found = snap
                .machine
                .sessions
                .get(session)
                .and_then(|s| s.agents.get(agent).map(|a| (s, a)));
            match found.and_then(|(s, a)| view::call_detail(a, call, now).map(|d| (s, a, d))) {
                Some((s, a, d)) => Body::Call(Box::new(call_body(s, a, call, d))),
                None => gone("call"),
            }
        }
    }
}

fn machine(snap: &Snapshot, prefs: &Prefs, show_exited: bool, now: Time) -> MachineBody {
    let mut galleries: Vec<Gallery> = Vec::new();
    for p in project::projects(snap) {
        let mut chambers: Vec<Chamber> = snap
            .machine
            .sessions
            .values()
            .filter(|s| {
                s.status != antfarm_core::SessionStatus::Exited
                    && project::repo_key(s).display().to_string() == p.key
            })
            .map(|s| {
                let (tool, target) = view::session_current(s);
                Chamber {
                    id: s.id.clone(),
                    name: view::session_name(s),
                    class: Class::of_session(s.status),
                    state: s.status.label().to_owned(),
                    waiting: match s.status {
                        antfarm_core::SessionStatus::Waiting(k) => Some(k),
                        _ => None,
                    },
                    for_secs: secs(s.status_since, now),
                    tool,
                    target,
                    source: view::source_label(s),
                    cost: antfarm_view::fmt::money(&s.cost),
                    agents: view::agent_lines(s, now)
                        .into_iter()
                        .filter(|a| a.depth > 0)
                        // The map shows what is alive, plus what finished in the last ten
                        // minutes; still and exited agents would only crowd it.
                        .filter(|a| match a.class {
                            Class::Working | Class::Waiting => true,
                            Class::Finished => recently_finished(s, &a.id, now),
                            _ => false,
                        })
                        .take(12)
                        .collect(),
                }
            })
            .collect();
        chambers.sort_by_key(|c| antfarm_view::fmt::rank_class(c.class));
        galleries.push(Gallery {
            waiting: chambers
                .iter()
                .filter(|c| c.class == Class::Waiting)
                .count(),
            working: chambers
                .iter()
                .filter(|c| c.class == Class::Working)
                .count(),
            key: p.key,
            name: p.name,
            chambers,
        });
    }
    galleries.sort_by_key(|g| (g.waiting == 0, g.working == 0));
    let mut feed: Vec<FeedItem> = snap
        .machine
        .sessions
        .values()
        .flat_map(|s| {
            view::call_lines(s, 30).into_iter().map(move |c| FeedItem {
                session: s.id.clone(),
                session_name: view::session_name(s),
                call: c,
            })
        })
        .collect();
    feed.sort_by_key(|f| std::cmp::Reverse(f.call.at));
    feed.truncate(40);
    let agents: Vec<&antfarm_core::Agent> = snap
        .machine
        .sessions
        .values()
        .flat_map(|s| s.agents.values())
        .collect();
    let histogram = view::spark(&agents, now);
    MachineBody {
        rows: view::table_rows(snap, prefs, show_exited, now),
        galleries,
        feed,
        calls_per_min: histogram.iter().rev().take(5).sum::<u32>() as f64 / 5.0,
        histogram,
    }
}

/// Finished agents linger on the map for ten minutes.
fn recently_finished(s: &antfarm_core::Session, id: &AgentId, now: Time) -> bool {
    s.agents
        .get(id)
        .and_then(|a| a.finished)
        .is_some_and(|f| (now - f).num_minutes() < 10)
}

fn session(s: &antfarm_core::Session, prefs: &Prefs, notify: Notify, now: Time) -> SessionBody {
    let (timeline_start, lanes) = view::lanes(s, now);
    let last_call = view::call_lines(s, 1).first().and_then(|c| c.at);
    let active: i64 = s
        .agents
        .get(&s.root)
        .map(|a| {
            a.all_calls()
                .filter_map(|c| Some((c.finished? - c.at?).num_seconds().max(0)))
                .sum()
        })
        .unwrap_or(0);
    SessionBody {
        id: s.id.clone(),
        name: view::session_name(s),
        project: project_name(s),
        project_key: project::repo_key(s).display().to_string(),
        branch: s.branch.clone(),
        tree: project::tree_key(s).display().to_string(),
        source: view::source_label(s),
        pid: s.pid,
        class: Class::of_session(s.status),
        status: s.status.label().to_owned(),
        for_secs: secs(s.status_since, now),
        uptime_secs: secs(s.started, now),
        active_secs: Some(active),
        active_partial: s.history != antfarm_core::HistoryState::Complete,
        last_call_secs: secs(last_call, now),
        task: s.task.clone(),
        cost: antfarm_view::fmt::money(&s.cost),
        usd_per_min: s.cost.usd_per_min,
        tokens_in: s.usage.input + s.usage.cache_creation + s.usage.cache_read,
        tokens_out: s.usage.output,
        context_pct: s.context.pct,
        calls: view::call_count(s),
        waiting: view::waiting(s, now),
        agents: view::agent_lines(s, now),
        timeline_start: timeline_start.map(|(a, _)| a),
        lanes,
        call_lines: view::call_lines(s, 200),
        files: s
            .files_touched
            .iter()
            .rev()
            .map(|f| FileItem {
                path: f.path.clone(),
                added: f.added,
                removed: f.removed,
                tool: f.tool.clone(),
                at: f.at,
            })
            .collect(),
        notify,
        muted: prefs.muted.contains(&s.id),
        snoozed: prefs.snoozed(&s.id, now),
    }
}

/// The swimlane window: the route's own when it has one, else the last fifteen minutes.
#[allow(clippy::too_many_arguments)]
fn lanes_body(
    s: &antfarm_core::Session,
    start: Option<Time>,
    end: Option<Time>,
    playhead: Option<Time>,
    span: Option<i64>,
    width: u32,
    now: Time,
) -> LanesBody {
    // Following keeps the window's width and slides its start with now.
    let following = end.is_none();
    let end = end.unwrap_or(now).min(now);
    let start = if following {
        end - chrono::Duration::seconds(span.unwrap_or(900).clamp(30, 30 * 86_400))
    } else {
        start
            .filter(|st| *st < end)
            .unwrap_or(end - chrono::Duration::minutes(15))
    };
    let w = Window { start, end };
    let playhead = playhead.unwrap_or(end).clamp(start, end);
    let (lanes, hidden_lanes) = lanes::lane_intervals(s, &w, now, width);
    LanesBody {
        id: s.id.clone(),
        name: view::session_name(s),
        start,
        end,
        playhead,
        following,
        history_start: lanes::history_start(s),
        history_note: lanes::history_note(s),
        lanes,
        hidden_lanes,
        agents_at: lanes::agents_at(s, playhead, now),
        calls_at: view::call_lines_before(s, 100, Some(playhead)),
        waiting: view::waiting(s, now),
    }
}

fn agent_body(s: &antfarm_core::Session, a: &antfarm_core::Agent, now: Time) -> AgentBody {
    AgentBody {
        session: s.id.clone(),
        session_name: view::session_name(s),
        id: a.id.clone(),
        read: a.read || a.is_root(),
        name: if a.parent.is_none() {
            view::session_name(s)
        } else {
            view::agent_name(a)
        },
        kind: a.kind.clone(),
        class: Class::of_agent(a.status),
        state: format!("{:?}", a.status).to_lowercase(),
        depth: a.depth,
        background: a.background,
        model: a.model.clone(),
        effort: a.effort.clone(),
        cost: a.cost_usd,
        context_pct: a.context.pct,
        elapsed_secs: a
            .started
            .map(|st| (a.finished.unwrap_or(now) - st).num_seconds().max(0)),
        brief: a.description.clone().or_else(|| s.task.clone()),
        parent: a.parent.as_ref().and_then(|p| s.agents.get(p)).map(|p| {
            if p.parent.is_none() {
                view::session_name(s)
            } else {
                view::agent_name(p)
            }
        }),
        calls: view::waterfall(a),
    }
}

/// How much of a background command's output the call screen shows.
pub const OUTPUT_TAIL_BYTES: u64 = 32 << 10;

fn call_body(
    s: &antfarm_core::Session,
    a: &antfarm_core::Agent,
    call: &ToolCallId,
    detail: CallDetail,
) -> CallBody {
    // Only a background task's output file is read: a path under a `tasks` directory
    // ending in `.output`, as Claude Code writes them.
    let output_tail = detail
        .background_run
        .as_ref()
        .map(|b| std::path::PathBuf::from(&b.output))
        .filter(|p| {
            p.extension().is_some_and(|e| e == "output")
                && p.parent()
                    .and_then(|d| d.file_name())
                    .is_some_and(|d| d == "tasks")
        })
        .and_then(|p| antfarm_core::tail::tail_text(&p, OUTPUT_TAIL_BYTES).ok())
        .map(|t| view::mask_secrets(&t));
    // The call this agent finished just before this one.
    let started = detail.started;
    let last_completed = view::call_lines(s, 200).into_iter().find(|c| {
        c.agent == a.id
            && c.call != *call
            && c.status != antfarm_core::ResultStatus::Pending
            && c.at <= started
    });
    CallBody {
        session: s.id.clone(),
        session_name: view::session_name(s),
        agent: a.id.clone(),
        agent_name: if a.parent.is_none() {
            view::session_name(s)
        } else {
            view::agent_name(a)
        },
        call: call.clone(),
        detail,
        output_tail,
        last_completed,
    }
}
