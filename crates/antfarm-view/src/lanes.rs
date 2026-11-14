//! The swimlane view model: which agents have a lane in a time window, what each column
//! of each lane shows, the axis, and the session's state at the playhead. Pure functions
//! of a session, a window, and `now`.

use antfarm_core::{
    Agent, AgentId, AgentStatus, HistoryState, ResultStatus, Session, Time, ToolCallId,
    ToolCategory,
};
use chrono::Duration;

use super::view::{self, Class};

/// Zoom levels, narrowest first.
pub const PRESETS: &[i64] = &[60, 300, 900, 3600, 7200, 21_600, 86_400];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Span {
    /// One of [`PRESETS`], in seconds.
    Secs(i64),
    /// Everything from the oldest kept history to now.
    Fit,
}

impl Span {
    pub fn label(self) -> String {
        match self {
            Span::Fit => "fit".into(),
            Span::Secs(s) if s < 3600 => format!("{}m", s / 60),
            Span::Secs(s) => format!("{}h", s / 3600),
        }
    }

    /// One step narrower, or the narrowest preset wider than `fit` when fitted.
    pub fn zoom_in(self, fit_secs: i64) -> Span {
        let cur = self.secs(fit_secs);
        PRESETS
            .iter()
            .rev()
            .find(|p| **p < cur)
            .map(|p| Span::Secs(*p))
            .unwrap_or(Span::Secs(PRESETS[0]))
    }

    pub fn zoom_out(self, fit_secs: i64) -> Span {
        let cur = self.secs(fit_secs);
        match PRESETS.iter().find(|p| **p > cur) {
            Some(p) if *p < fit_secs => Span::Secs(*p),
            _ => Span::Fit,
        }
    }

    pub fn secs(self, fit_secs: i64) -> i64 {
        match self {
            Span::Secs(s) => s,
            Span::Fit => fit_secs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Window {
    pub start: Time,
    pub end: Time,
}

impl Window {
    pub fn secs(&self) -> i64 {
        (self.end - self.start).num_seconds().max(1)
    }

    /// The window of `span` ending at `end`, never reaching past `now`.
    pub fn ending(span_secs: i64, end: Time, now: Time) -> Window {
        let end = end.min(now);
        Window {
            start: end - Duration::seconds(span_secs.max(1)),
            end,
        }
    }
}

/// Earliest moment the session has history for: the first kept call or agent start.
pub fn history_start(s: &Session) -> Option<Time> {
    s.agents
        .values()
        .flat_map(|a| {
            a.all_calls()
                .find_map(|c| c.at)
                .into_iter()
                .chain(a.started)
        })
        .min()
}

/// Seconds from the session's history start to `now`, at least a minute.
pub fn fit_secs(s: &Session, now: Time) -> i64 {
    history_start(s)
        .map(|h| (now - h).num_seconds())
        .unwrap_or(60)
        .max(60)
}

/// Column of `t` in a window `width` columns wide, or `None` outside it. Column `c`
/// covers the stretch ending at `(c + 1) / width` of the window, so the window's end is
/// the last column and one column's time earlier is the column before it.
pub fn col_of(t: Time, w: &Window, width: usize) -> Option<usize> {
    if width == 0 || t < w.start || t > w.end {
        return None;
    }
    let frac = (t - w.start).num_milliseconds() as f64
        / (w.end - w.start).num_milliseconds().max(1) as f64;
    Some(
        ((frac * width as f64).ceil() as usize)
            .saturating_sub(1)
            .min(width - 1),
    )
}

/// Column of `t`, clamped to the window's edges.
fn col_clamped(t: Time, w: &Window, width: usize) -> usize {
    if t <= w.start {
        0
    } else if t >= w.end {
        width.saturating_sub(1)
    } else {
        col_of(t, w, width).unwrap_or(0)
    }
}

/// The time at the middle of column `col`.
#[cfg(test)]
pub fn time_of(col: usize, w: &Window, width: usize) -> Time {
    let ms = (w.end - w.start).num_milliseconds() as f64 * (col as f64 + 0.5) / width.max(1) as f64;
    w.start + Duration::milliseconds(ms as i64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Cell {
    Empty,
    /// Alive, between calls.
    Life(Class),
    Call {
        category: ToolCategory,
        status: ResultStatus,
        waiting: bool,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LaneRow {
    pub agent: AgentId,
    pub label: String,
    pub depth: u8,
    pub class: Class,
    pub cells: Vec<Cell>,
    /// Where this agent was spawned, drawn as a marker.
    pub spawn_col: Option<usize>,
}

/// When an agent stops being alive, for drawing its life line.
fn life_end(a: &Agent, now: Time) -> Option<Time> {
    a.finished.or(match a.status {
        AgentStatus::Working | AgentStatus::Thinking | AgentStatus::Waiting(_) => Some(now),
        _ => a.last_write,
    })
}

/// Priority when calls share a column: what the human must act on, then failures, then
/// work still running, then whichever ran longer.
fn rank(status: ResultStatus, waiting: bool) -> u8 {
    match (waiting, status) {
        (true, _) => 4,
        (_, ResultStatus::Error) => 3,
        (_, ResultStatus::Pending) => 2,
        _ => 1,
    }
}

pub fn lane_rows(s: &Session, w: &Window, width: usize, now: Time) -> Vec<LaneRow> {
    if width == 0 {
        return Vec::new();
    }
    let blocking: Option<ToolCallId> = view::blocking_call(s)
        .filter(|_| s.status.needs_human())
        .map(|c| c.id.clone());
    view::tree_order(s)
        .into_iter()
        .filter_map(|a| {
            let start = a.started.or_else(|| a.all_calls().find_map(|c| c.at));
            let end = life_end(a, now).or(start);
            let overlaps =
                matches!((start, end), (Some(st), Some(e)) if st <= w.end && e >= w.start);
            if a.parent.is_some() && !overlaps {
                return None;
            }
            let class = Class::of_agent(a.status);
            let mut cells = vec![Cell::Empty; width];
            if let (Some(st), Some(e)) = (start, end)
                && overlaps
            {
                let (from, to) = (col_clamped(st, w, width), col_clamped(e, w, width));
                for c in cells.iter_mut().take(to + 1).skip(from) {
                    *c = Cell::Life(class);
                }
            }
            // (rank, duration) of whatever holds each column.
            let mut held = vec![(0u8, 0i64); width];
            for c in a.all_calls() {
                let Some(at) = c.at else { continue };
                let status = c.status;
                let stop = c.finished.unwrap_or(if status == ResultStatus::Pending {
                    now
                } else {
                    at
                });
                if stop < w.start || at > w.end {
                    continue;
                }
                let waiting = blocking.as_ref() == Some(c.call);
                let key = (rank(status, waiting), (stop - at).num_milliseconds());
                let (from, to) = (
                    col_clamped(at, w, width),
                    col_clamped(stop, w, width).max(col_clamped(at, w, width)),
                );
                for col in from..=to.min(width.saturating_sub(1)) {
                    if key > held[col] {
                        held[col] = key;
                        cells[col] = Cell::Call {
                            category: ToolCategory::of(c.name),
                            status,
                            waiting,
                        };
                    }
                }
            }
            Some(LaneRow {
                agent: a.id.clone(),
                label: if a.parent.is_none() {
                    view::session_name(s)
                } else {
                    view::agent_name(a)
                },
                depth: a.depth,
                class,
                cells,
                spawn_col: a
                    .parent
                    .as_ref()
                    .and(a.started)
                    .and_then(|t| col_of(t, w, width)),
            })
        })
        .collect()
}

/// Tick labels on round time steps, at least ten columns apart. Steps of a day or more
/// are labelled with the date.
pub fn axis(w: &Window, width: usize) -> Vec<(usize, String)> {
    const STEPS: &[i64] = &[
        1, 5, 10, 30, 60, 120, 300, 600, 900, 1200, 1800, 2400, 3600, 7200, 10_800, 21_600, 43_200,
        86_400, 172_800, 604_800, 1_209_600, 2_592_000,
    ];
    if width < 10 {
        return Vec::new();
    }
    let max_ticks = (width / 10).max(1) as i64;
    let step = STEPS
        .iter()
        .copied()
        .find(|s| w.secs() / s <= max_ticks)
        .unwrap_or(2_592_000);
    let first = (w.start.timestamp() + step - 1).div_euclid(step) * step;
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut t = first;
    while t <= w.end.timestamp() {
        let at = Time::from_timestamp(t, 0).unwrap_or(w.start);
        if let Some(c) = col_of(at, w, width) {
            let label = if step >= 86_400 {
                crate::fmt::date(at)
            } else if step < 60 {
                crate::fmt::clock(at)
            } else {
                crate::fmt::clock_hm(at)
            };
            if out
                .last()
                .is_none_or(|(prev, l)| c > prev + l.chars().count())
            {
                out.push((c, label));
            }
        }
        t += step;
    }
    out
}

/// What to say about history the session doesn't hold yet: backfill's progress while it
/// reads, or that it is off. `None` once the session's history is complete.
pub fn history_note(s: &Session) -> Option<String> {
    match s.history {
        HistoryState::Complete => None,
        HistoryState::Loading { read, total } => Some(format!(
            "loading older history {}%",
            (read * 100).checked_div(total).unwrap_or(0).min(99)
        )),
        HistoryState::Partial => Some("older history not loaded".into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum StateAt {
    NotStarted,
    Running,
    /// Blocked on a foreground sub-agent it spawned.
    WaitingOnAgents,
    /// Not finished, but nothing written for a long while before `t`.
    Still,
    Finished,
}

impl StateAt {
    pub fn label(self) -> &'static str {
        match self {
            StateAt::NotStarted => "not started",
            StateAt::Running => "working",
            StateAt::WaitingOnAgents => "waiting on agents",
            StateAt::Still => "still",
            StateAt::Finished => "finished",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AgentAt {
    pub agent: AgentId,
    pub label: String,
    pub depth: u8,
    pub state: StateAt,
}

/// Each agent's state at `t`, in tree order, leaving out agents not started yet.
pub fn agents_at(s: &Session, t: Time, now: Time) -> Vec<AgentAt> {
    view::tree_order(s)
        .into_iter()
        .filter_map(|a| {
            let state = match (a.started, life_end(a, now)) {
                (Some(st), _) if st > t => StateAt::NotStarted,
                (None, _) => StateAt::NotStarted,
                (_, Some(e)) if e < t && a.finished.is_some() => StateAt::Finished,
                _ if a.finished.is_none()
                    && matches!(a.status, AgentStatus::Still | AgentStatus::Exited)
                    && a.last_write.is_some_and(|lw| {
                        (t - lw).num_seconds() > antfarm_core::engine::status::STILL_AFTER_SECS
                    }) =>
                {
                    StateAt::Still
                }
                _ => {
                    // Blocked while one of its own foreground spawn calls is open at `t`.
                    // A background spawn returns at once, so it never blocks.
                    let blocked = a.all_calls().any(|c| {
                        antfarm_core::claude::subagents::SPAWN_TOOLS.contains(&c.name)
                            && c.at.is_some_and(|at| at <= t)
                            && c.finished.is_none_or(|f| f > t)
                    });
                    if blocked {
                        StateAt::WaitingOnAgents
                    } else {
                        StateAt::Running
                    }
                }
            };
            (state != StateAt::NotStarted).then(|| AgentAt {
                agent: a.id.clone(),
                label: if a.parent.is_none() {
                    view::session_name(s)
                } else {
                    view::agent_name(a)
                },
                depth: a.depth,
                state,
            })
        })
        .collect()
}

/// The start of the nearest call strictly before or after `t`.
pub fn neighbor_call(s: &Session, t: Time, forward: bool) -> Option<Time> {
    let starts = s
        .agents
        .values()
        .flat_map(|a| a.all_calls())
        .filter_map(|c| c.at);
    if forward {
        starts.filter(|a| *a > t).min()
    } else {
        starts.filter(|a| *a < t).max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use antfarm_core::Activity;
    use antfarm_core::{SessionId, SessionStatus};

    fn t(s: &str) -> Time {
        s.parse().unwrap()
    }

    fn call(id: &str, name: &str, at: &str, end: Option<&str>, status: ResultStatus) -> Activity {
        Activity::Call {
            call: ToolCallId::from(id),
            name: name.into(),
            summary: id.into(),
            at: Some(t(at)),
            finished: end.map(t),
            status,
            input: None,
            why: None,
            background: None,
            lines: Default::default(),
        }
    }

    fn session() -> Session {
        let mut s = Session::new(SessionId::from("s"), "/w/app".into());
        s.name = Some("app".into());
        s.status = SessionStatus::Working;
        let root = s.root.clone();
        let r = &mut s.agents[&root];
        r.started = Some(t("2026-11-03T10:00:00Z"));
        r.status = AgentStatus::Working;
        r.activity.push_back(call(
            "a",
            "Read",
            "2026-11-03T10:00:00Z",
            Some("2026-11-03T10:00:01Z"),
            ResultStatus::Ok,
        ));
        r.activity.push_back(call(
            "b",
            "Bash",
            "2026-11-03T10:01:00Z",
            Some("2026-11-03T10:03:00Z"),
            ResultStatus::Ok,
        ));
        r.activity.push_back(call(
            "c",
            "Grep",
            "2026-11-03T10:02:00Z",
            Some("2026-11-03T10:02:05Z"),
            ResultStatus::Error,
        ));
        r.activity.push_back(call(
            "d",
            "Bash",
            "2026-11-03T10:08:00Z",
            None,
            ResultStatus::Pending,
        ));
        let mut sub = Agent::new(AgentId::from("k"), "Explore");
        sub.parent = Some(root.clone());
        sub.depth = 1;
        sub.started = Some(t("2026-11-03T10:04:00Z"));
        sub.finished = Some(t("2026-11-03T10:06:00Z"));
        sub.status = AgentStatus::Finished;
        sub.description = Some("map callers".into());
        sub.activity.push_back(call(
            "e",
            "Grep",
            "2026-11-03T10:04:10Z",
            Some("2026-11-03T10:04:20Z"),
            ResultStatus::Ok,
        ));
        let mut old = Agent::new(AgentId::from("o"), "Explore");
        old.parent = Some(root.clone());
        old.depth = 1;
        old.started = Some(t("2026-11-03T08:00:00Z"));
        old.finished = Some(t("2026-11-03T08:10:00Z"));
        old.status = AgentStatus::Finished;
        s.agents[&root].children = vec![AgentId::from("o"), AgentId::from("k")];
        s.agents.insert(sub.id.clone(), sub);
        s.agents.insert(old.id.clone(), old);
        s
    }

    #[test]
    fn columns_map_both_ways_and_clamp() {
        let w = Window {
            start: t("2026-11-03T10:00:00Z"),
            end: t("2026-11-03T10:10:00Z"),
        };
        assert_eq!(col_of(w.start, &w, 60), Some(0));
        assert_eq!(col_of(w.end, &w, 60), Some(59));
        assert_eq!(col_of(t("2026-11-03T10:05:00Z"), &w, 60), Some(29));
        // One column before the end is the column before the last.
        assert_eq!(col_of(w.end - Duration::seconds(10), &w, 60), Some(58));
        assert_eq!(col_of(t("2026-11-03T09:59:59Z"), &w, 60), None);
        assert_eq!(col_of(w.start, &w, 0), None);
        let mid = time_of(30, &w, 60);
        assert_eq!(col_of(mid, &w, 60), Some(30));
    }

    #[test]
    fn lanes_bars_priority_and_running_calls() {
        let s = session();
        let now = t("2026-11-03T10:10:00Z");
        let w = Window {
            start: t("2026-11-03T10:00:00Z"),
            end: now,
        };
        let rows = lane_rows(&s, &w, 60, now);
        assert_eq!(
            rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
            ["app", "map callers"],
            "the agent from 8am is outside the window"
        );
        let root = &rows[0].cells;
        // The two-minute Bash covers 10:01 to 10:03; the failed Grep at 10:02 wins its column.
        assert!(matches!(
            root[7],
            Cell::Call {
                category: ToolCategory::Shell,
                status: ResultStatus::Ok,
                ..
            }
        ));
        assert!(matches!(
            root[12],
            Cell::Call {
                status: ResultStatus::Error,
                ..
            }
        ));
        // The running Bash from 10:08 reaches now.
        assert!(matches!(
            root[59],
            Cell::Call {
                status: ResultStatus::Pending,
                ..
            }
        ));
        // Between calls the root is alive.
        assert!(matches!(root[25], Cell::Life(Class::Working)));
        // A one-second Read still gets a column.
        assert!(matches!(
            root[0],
            Cell::Call {
                category: ToolCategory::Read,
                ..
            }
        ));
        let sub = &rows[1];
        assert_eq!(sub.spawn_col, Some(23));
        assert!(matches!(sub.cells[22], Cell::Empty));
        assert!(matches!(sub.cells[40], Cell::Empty), "finished at 10:06");
    }

    #[test]
    fn axis_ticks_are_round_and_spaced() {
        let w = Window {
            start: t("2026-11-03T10:00:30Z"),
            end: t("2026-11-03T10:15:30Z"),
        };
        let ticks = axis(&w, 100);
        assert!(ticks.len() >= 3 && ticks.len() <= 10, "{ticks:?}");
        assert!(ticks.windows(2).all(|p| p[1].0 >= p[0].0 + 10));
        // Two-minute steps: every label is an even minute.
        assert!(
            ticks
                .iter()
                .all(|(_, l)| l[3..].parse::<u32>().is_ok_and(|m| m % 2 == 0)),
            "{ticks:?}"
        );
        assert!(axis(&w, 5).is_empty());
        for preset in PRESETS {
            let end = t("2026-11-03T10:17:23Z");
            let ticks = axis(&Window::ending(*preset, end, end), 97);
            assert!(ticks.len() >= 2, "{preset}: {ticks:?}");
            assert!(
                ticks.windows(2).all(|p| p[0].1 != p[1].1),
                "{preset}: repeated labels {ticks:?}"
            );
        }
        // A nine-day window is labelled by date, never ten identical times.
        let end = t("2026-11-03T04:33:05Z");
        let ticks = axis(&Window::ending(9 * 86_400, end, end), 97);
        assert!(
            ticks.len() >= 2 && ticks.windows(2).all(|p| p[0].1 != p[1].1),
            "{ticks:?}"
        );
    }

    #[test]
    fn state_at_a_moment_and_call_cutoff() {
        let mut s = session();
        let root = s.root.clone();
        // The root's foreground spawn of k is open from 10:03:50 until k returns at 10:06.
        s.agents[&root].activity.push_back(call(
            "sp",
            "Agent",
            "2026-11-03T10:03:50Z",
            Some("2026-11-03T10:06:00Z"),
            ResultStatus::Ok,
        ));
        let now = t("2026-11-03T10:10:00Z");
        let at = agents_at(&s, t("2026-11-03T10:05:00Z"), now);
        let states: Vec<_> = at.iter().map(|a| (a.label.as_str(), a.state)).collect();
        assert_eq!(
            states,
            [
                ("app", StateAt::WaitingOnAgents),
                ("o", StateAt::Finished),
                ("map callers", StateAt::Running)
            ]
        );
        let at = agents_at(&s, t("2026-11-03T10:07:00Z"), now);
        assert!(
            at.iter()
                .any(|a| a.label == "map callers" && a.state == StateAt::Finished)
        );
        let calls = view::call_lines_before(&s, 10, Some(t("2026-11-03T10:02:00Z")));
        assert_eq!(
            calls.iter().map(|c| c.call.as_str()).collect::<Vec<_>>(),
            ["c", "b", "a"]
        );
        assert_eq!(
            neighbor_call(&s, t("2026-11-03T10:02:00Z"), true),
            Some(t("2026-11-03T10:03:50Z"))
        );
        assert_eq!(
            neighbor_call(&s, t("2026-11-03T10:02:00Z"), false),
            Some(t("2026-11-03T10:01:00Z"))
        );
    }

    #[test]
    fn zoom_steps_through_presets_and_fit() {
        let fit = 5 * 3600;
        assert_eq!(Span::Fit.zoom_in(fit), Span::Secs(3600 * 2));
        assert_eq!(Span::Secs(900).zoom_in(fit), Span::Secs(300));
        assert_eq!(Span::Secs(60).zoom_in(fit), Span::Secs(60));
        assert_eq!(Span::Secs(3600).zoom_out(fit), Span::Secs(7200));
        assert_eq!(Span::Secs(7200).zoom_out(fit), Span::Fit);
        assert_eq!(Span::Secs(900).label(), "15m");
        assert_eq!(Span::Secs(7200).label(), "2h");
    }
}

/// One call as a time interval, for drawing bars at any zoom.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Interval {
    /// How many calls this interval stands for; calls too close together to tell apart
    /// at the drawing width are merged.
    pub count: u32,
    pub call: ToolCallId,
    pub name: String,
    pub summary: String,
    pub category: ToolCategory,
    pub status: ResultStatus,
    pub start: Time,
    /// The finish time, or now while it runs.
    pub end: Time,
    pub waiting: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LaneIntervals {
    pub agent: AgentId,
    pub label: String,
    pub depth: u8,
    pub class: Class,
    pub background: bool,
    pub parent: Option<AgentId>,
    /// When the agent was alive, clipped to nothing outside the window.
    pub life: Option<(Time, Time)>,
    pub spawned: Option<Time>,
    pub calls: Vec<Interval>,
}

/// Most lanes a vector view draws; the rest are counted, not drawn.
pub const MAX_LANES: usize = 100;

/// Lanes as intervals overlapping the window, in tree order, for a vector renderer.
/// Calls closer together than two pixels at `px` wide are merged, so a long window on a
/// busy session stays small, and lanes past [`MAX_LANES`] are left out and counted.
pub fn lane_intervals(s: &Session, w: &Window, now: Time, px: u32) -> (Vec<LaneIntervals>, usize) {
    let mut lanes = lane_intervals_all(s, w, now);
    let bucket = chrono::Duration::milliseconds(
        (w.end - w.start).num_milliseconds() * 2 / i64::from(px.max(50)),
    );
    for l in &mut lanes {
        l.calls = coalesce(std::mem::take(&mut l.calls), bucket);
    }
    // Sub-agents with nothing in the window and no life in it add nothing.
    lanes.retain(|l| l.parent.is_none() || !l.calls.is_empty() || l.life.is_some());
    let hidden = lanes.len().saturating_sub(MAX_LANES);
    lanes.truncate(MAX_LANES);
    (lanes, hidden)
}

fn worse(a: ResultStatus, b: ResultStatus) -> ResultStatus {
    let rank = |s: ResultStatus| match s {
        ResultStatus::Error => 2,
        ResultStatus::Pending => 1,
        ResultStatus::Ok => 0,
    };
    if rank(b) > rank(a) { b } else { a }
}

fn coalesce(mut calls: Vec<Interval>, bucket: chrono::Duration) -> Vec<Interval> {
    calls.sort_by_key(|c| c.start);
    let mut out: Vec<Interval> = Vec::with_capacity(calls.len());
    for c in calls {
        match out.last_mut() {
            Some(last) if c.start - last.end < bucket && !c.waiting && !last.waiting => {
                last.end = last.end.max(c.end);
                last.count += c.count;
                last.status = worse(last.status, c.status);
                last.summary = format!("{} calls", last.count);
            }
            _ => out.push(c),
        }
    }
    out
}

fn lane_intervals_all(s: &Session, w: &Window, now: Time) -> Vec<LaneIntervals> {
    let blocking: Option<ToolCallId> = view::blocking_call(s)
        .filter(|_| s.status.needs_human())
        .map(|c| c.id.clone());
    view::tree_order(s)
        .into_iter()
        .filter_map(|a| {
            let start = a.started.or_else(|| a.all_calls().find_map(|c| c.at));
            let end = life_end(a, now).or(start);
            let overlaps =
                matches!((start, end), (Some(st), Some(e)) if st <= w.end && e >= w.start);
            if a.parent.is_some() && !overlaps {
                return None;
            }
            let calls = a
                .all_calls()
                .filter_map(|c| {
                    let at = c.at?;
                    let stop = c.finished.unwrap_or(if c.status == ResultStatus::Pending {
                        now
                    } else {
                        at
                    });
                    (stop >= w.start && at <= w.end).then(|| Interval {
                        count: 1,
                        call: c.call.clone(),
                        name: c.name.to_owned(),
                        summary: c.summary.to_owned(),
                        category: ToolCategory::of(c.name),
                        status: c.status,
                        start: at,
                        end: stop.max(at),
                        waiting: blocking.as_ref() == Some(c.call),
                    })
                })
                .collect();
            Some(LaneIntervals {
                agent: a.id.clone(),
                label: if a.parent.is_none() {
                    view::session_name(s)
                } else {
                    view::agent_name(a)
                },
                depth: a.depth,
                class: Class::of_agent(a.status),
                background: a.background,
                parent: a.parent.clone(),
                life: start.zip(end).filter(|_| overlaps),
                spawned: a.parent.as_ref().and(a.started),
                calls,
            })
        })
        .collect()
}

#[cfg(test)]
mod interval_tests {
    use super::*;

    #[test]
    fn close_calls_merge_and_lanes_are_capped() {
        let t = |m: i64| {
            "2026-11-03T10:00:00Z".parse::<Time>().unwrap() + chrono::Duration::milliseconds(m)
        };
        let iv = |m: i64, d: i64, st: ResultStatus| Interval {
            count: 1,
            call: ToolCallId::from(format!("c{m}")),
            name: "Bash".into(),
            summary: "x".into(),
            category: ToolCategory::Shell,
            status: st,
            start: t(m),
            end: t(m + d),
            waiting: false,
        };
        let merged = coalesce(
            vec![
                iv(0, 10, ResultStatus::Ok),
                iv(50, 10, ResultStatus::Error),
                iv(5_000, 10, ResultStatus::Ok),
            ],
            chrono::Duration::milliseconds(100),
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(
            (merged[0].count, merged[0].status),
            (2, ResultStatus::Error)
        );
        assert_eq!(merged[0].summary, "2 calls");
    }
}
