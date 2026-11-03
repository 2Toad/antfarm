//! UI state and key handling. Everything that changes the screen goes through
//! [`App::handle_key`]; side effects come back as [`Action`]s for the run loop.

use antfarm_core::{AgentId, Session, SessionId, Snapshot, Time, ToolCallId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::lanes::{self, Span, Window};
use super::view::{self, Prefs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Machine,
    Lanes {
        session: SessionId,
    },
    Session {
        session: SessionId,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    Agents,
    #[default]
    Calls,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    GoTo(SessionId),
    Copy(SessionId),
}

pub const SNOOZE_MINUTES: i64 = 10;
const TOAST_SECS: i64 = 3;
const PAGE: usize = 10;

#[derive(Debug)]
pub struct App {
    pub stack: Vec<Screen>,
    pub prefs: Prefs,
    pub show_exited: bool,
    /// The machine table's selection, by identity so it survives reordering.
    pub machine_key: Option<(SessionId, Option<AgentId>)>,
    pub machine_index: usize,
    pub focus: Focus,
    pub agent_index: usize,
    pub call_index: usize,
    pub water_index: usize,
    pub scroll: u16,
    pub editing_filter: bool,
    pub help: bool,
    pub toast: Option<(String, Time)>,
    pub quit: bool,
    pub lanes: LanesState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LanesFocus {
    #[default]
    Lanes,
    Calls,
}

/// Where the swimlane view is looking. While `follow` is on, the window ends at now and
/// the playhead sits on now.
#[derive(Debug, Clone)]
pub struct LanesState {
    pub span: Span,
    pub follow: bool,
    pub end: Time,
    pub playhead: Time,
    pub lane_index: usize,
    pub call_index: usize,
    pub focus: LanesFocus,
    /// Columns of the bar area at the last draw, for moving the playhead by one.
    pub width: usize,
}

impl Default for LanesState {
    fn default() -> Self {
        Self {
            span: Span::Secs(DEFAULT_SPAN),
            follow: true,
            end: Time::UNIX_EPOCH,
            playhead: Time::UNIX_EPOCH,
            lane_index: 0,
            call_index: 0,
            focus: LanesFocus::Lanes,
            width: 80,
        }
    }
}

/// The swimlane view opens on the last fifteen minutes.
pub const DEFAULT_SPAN: i64 = 900;

impl LanesState {
    /// The window and playhead as of `now`.
    pub fn window(&self, s: &Session, now: Time) -> (Window, Time) {
        let fit = lanes::fit_secs(s, now);
        let secs = self.span.secs(fit);
        let end = if self.follow || self.span == Span::Fit {
            now
        } else {
            self.end
        };
        let w = Window::ending(secs, end, now);
        let playhead = if self.follow {
            now
        } else {
            self.playhead.clamp(w.start, w.end)
        };
        (w, playhead)
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            stack: vec![Screen::Machine],
            prefs: Prefs {
                tree: true,
                ..Prefs::default()
            },
            show_exited: false,
            machine_key: None,
            machine_index: 0,
            focus: Focus::Calls,
            agent_index: 0,
            call_index: 0,
            water_index: 0,
            scroll: 0,
            editing_filter: false,
            help: false,
            toast: None,
            quit: false,
            lanes: LanesState::default(),
        }
    }
}

fn step(i: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (i as isize + delta).clamp(0, len as isize - 1) as usize
}

impl App {
    pub fn screen(&self) -> &Screen {
        self.stack
            .last()
            .expect("the machine screen is never popped")
    }

    pub fn toast(&mut self, msg: impl Into<String>, now: Time) {
        self.toast = Some((msg.into(), now + chrono::Duration::seconds(TOAST_SECS)));
    }

    pub fn live_toast(&self, now: Time) -> Option<&str> {
        self.toast
            .as_ref()
            .filter(|(_, until)| *until > now)
            .map(|(m, _)| m.as_str())
    }

    /// The session the current screen is about, or the machine selection's session.
    pub fn focused_session(&self, snap: &Snapshot, now: Time) -> Option<SessionId> {
        match self.screen() {
            Screen::Machine => self.machine_selection(snap, now).map(|(s, _)| s),
            Screen::Session { session }
            | Screen::Lanes { session }
            | Screen::Agent { session, .. }
            | Screen::Call { session, .. } => Some(session.clone()),
        }
    }

    /// Re-finds the machine selection in the current rows.
    pub fn sync_machine(&mut self, rows: &[view::Row]) {
        if let Some(key) = &self.machine_key
            && let Some(i) = rows
                .iter()
                .position(|r| (&r.session, &r.agent) == (&key.0, &key.1))
        {
            self.machine_index = i;
        }
        self.machine_index = self.machine_index.min(rows.len().saturating_sub(1));
        self.machine_key = rows
            .get(self.machine_index)
            .map(|r| (r.session.clone(), r.agent.clone()));
    }

    pub fn machine_selection(
        &self,
        snap: &Snapshot,
        now: Time,
    ) -> Option<(SessionId, Option<AgentId>)> {
        let rows = view::table_rows(snap, &self.prefs, self.show_exited, now);
        rows.get(self.machine_index.min(rows.len().saturating_sub(1)))
            .map(|r| (r.session.clone(), r.agent.clone()))
    }

    fn push(&mut self, s: Screen) {
        self.stack.push(s);
        self.scroll = 0;
        self.water_index = 0;
    }

    pub fn handle_key(&mut self, key: KeyEvent, snap: &Snapshot, now: Time) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return None;
        }
        if self.editing_filter {
            match key.code {
                KeyCode::Enter => self.editing_filter = false,
                KeyCode::Esc => {
                    self.editing_filter = false;
                    self.prefs.filter.clear();
                }
                KeyCode::Backspace => {
                    self.prefs.filter.pop();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.prefs.filter.push(c)
                }
                _ => {}
            }
            return None;
        }
        if self.help {
            self.help = false;
            return None;
        }
        if let Screen::Lanes { session } = self.screen().clone()
            && let Some(s) = snap.machine.sessions.get(&session)
            && self.lanes_key(key, s, &session, now)
        {
            return None;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('l') | KeyCode::Char('L') => {
                if let Some(session) = self.focused_session(snap, now) {
                    self.lanes = LanesState::default();
                    self.push(Screen::Lanes { session });
                }
            }
            KeyCode::Char('?') | KeyCode::F(1) => self.help = true,
            KeyCode::Esc => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                    self.scroll = 0;
                } else if !self.prefs.filter.is_empty() {
                    self.prefs.filter.clear();
                }
            }
            KeyCode::Char('/') => {
                if !matches!(self.screen(), Screen::Machine) {
                    self.stack.truncate(1);
                }
                self.editing_filter = true;
            }
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::F(6) => {
                self.prefs.sort = self.prefs.sort.next()
            }
            KeyCode::Char('t') | KeyCode::Char('T') | KeyCode::F(5) => {
                self.prefs.tree = !self.prefs.tree
            }
            KeyCode::Char('f') => self.prefs.show_finished = !self.prefs.show_finished,
            KeyCode::Char('a') => self.show_exited = !self.show_exited,
            KeyCode::Char('n') | KeyCode::Char('N') => self.next_waiting(snap, now),
            KeyCode::Char('g') | KeyCode::Char('G') => {
                return self.focused_session(snap, now).map(Action::GoTo);
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                return self.focused_session(snap, now).map(Action::Copy);
            }
            KeyCode::Char('z') | KeyCode::Char('Z') => self.snooze(snap, now),
            KeyCode::Char('m') | KeyCode::Char('M') => self.mute(snap, now),
            KeyCode::Tab => {
                self.focus = if self.focus == Focus::Calls {
                    Focus::Agents
                } else {
                    Focus::Calls
                }
            }
            KeyCode::Enter => self.drill(snap, now),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1, snap, now),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1, snap, now),
            KeyCode::PageUp => self.move_by(-(PAGE as isize), snap, now),
            KeyCode::PageDown => self.move_by(PAGE as isize, snap, now),
            KeyCode::Home => self.move_by(-100_000, snap, now),
            KeyCode::End => self.move_by(100_000, snap, now),
            _ => {}
        }
        None
    }

    fn move_by(&mut self, d: isize, snap: &Snapshot, now: Time) {
        match self.screen().clone() {
            Screen::Machine => {
                let rows = view::table_rows(snap, &self.prefs, self.show_exited, now);
                self.machine_index = step(self.machine_index, d, rows.len());
                self.machine_key = rows
                    .get(self.machine_index)
                    .map(|r| (r.session.clone(), r.agent.clone()));
            }
            Screen::Session { session } => {
                let Some(s) = snap.machine.sessions.get(&session) else {
                    return;
                };
                match self.focus {
                    Focus::Agents => {
                        self.agent_index = step(self.agent_index, d, view::tree_order(s).len())
                    }
                    Focus::Calls => {
                        self.call_index =
                            step(self.call_index, d, view::call_lines(s, CALL_LIMIT).len())
                    }
                }
            }
            Screen::Agent { session, agent } => {
                let n = snap
                    .machine
                    .sessions
                    .get(&session)
                    .and_then(|s| s.agents.get(&agent))
                    .map(|a| view::waterfall(a).len())
                    .unwrap_or(0);
                self.water_index = step(self.water_index, d, n);
            }
            Screen::Call { .. } => {
                self.scroll = (self.scroll as isize + d).clamp(0, u16::MAX as isize) as u16
            }
            Screen::Lanes { .. } => {}
        }
    }

    fn drill(&mut self, snap: &Snapshot, now: Time) {
        match self.screen().clone() {
            Screen::Machine => {
                let Some((session, agent)) = self.machine_selection(snap, now) else {
                    return;
                };
                self.agent_index = 0;
                self.call_index = 0;
                self.focus = Focus::Calls;
                self.push(Screen::Session {
                    session: session.clone(),
                });
                if let Some(agent) = agent {
                    self.push(Screen::Agent { session, agent });
                }
            }
            Screen::Session { session } => {
                let Some(s) = snap.machine.sessions.get(&session) else {
                    return;
                };
                match self.focus {
                    Focus::Agents => {
                        if let Some(a) = view::tree_order(s).get(self.agent_index) {
                            let agent = a.id.clone();
                            self.push(Screen::Agent { session, agent });
                        }
                    }
                    Focus::Calls => {
                        if let Some(c) = view::call_lines(s, CALL_LIMIT).get(self.call_index) {
                            let (agent, call) = (c.agent.clone(), c.call.clone());
                            self.push(Screen::Call {
                                session,
                                agent,
                                call,
                            });
                        }
                    }
                }
            }
            Screen::Agent { session, agent } => {
                let Some(a) = snap
                    .machine
                    .sessions
                    .get(&session)
                    .and_then(|s| s.agents.get(&agent))
                else {
                    return;
                };
                if let Some(w) = view::waterfall(a).get(self.water_index) {
                    let call = w.call.clone();
                    self.push(Screen::Call {
                        session,
                        agent,
                        call,
                    });
                }
            }
            Screen::Call { .. } | Screen::Lanes { .. } => {}
        }
    }

    fn next_waiting(&mut self, snap: &Snapshot, now: Time) {
        let waiting = view::needs_you(snap, &self.prefs, now);
        if waiting.is_empty() {
            self.toast("nothing is waiting for you", now);
            return;
        }
        let current = self.focused_session(snap, now);
        let i = current
            .and_then(|c| waiting.iter().position(|w| w.session == c))
            .map(|i| (i + 1) % waiting.len())
            .unwrap_or(0);
        let target = waiting[i].session.clone();
        match self.screen() {
            Screen::Machine => {
                self.machine_key = Some((target, None));
                let rows = view::table_rows(snap, &self.prefs, self.show_exited, now);
                self.sync_machine(&rows);
            }
            _ => {
                self.stack.truncate(1);
                self.focus = Focus::Calls;
                self.call_index = 0;
                self.push(Screen::Session { session: target });
            }
        }
    }

    fn snooze(&mut self, snap: &Snapshot, now: Time) {
        let Some(id) = self.focused_session(snap, now) else {
            return;
        };
        if !snap
            .machine
            .sessions
            .get(&id)
            .is_some_and(|s| s.status.needs_human())
        {
            self.toast("only a waiting session can be snoozed", now);
            return;
        }
        self.prefs
            .snoozed
            .insert(id, now + chrono::Duration::minutes(SNOOZE_MINUTES));
        self.toast(format!("snoozed for {SNOOZE_MINUTES} minutes"), now);
    }

    fn mute(&mut self, snap: &Snapshot, now: Time) {
        let Some(id) = self.focused_session(snap, now) else {
            return;
        };
        if self.prefs.muted.remove(&id) {
            self.toast("unmuted", now);
        } else {
            self.prefs.muted.insert(id);
            self.toast("muted: dimmed and sorted last", now);
        }
    }
}

/// How many calls the session screen lists.
pub const CALL_LIMIT: usize = 200;

impl App {
    /// Keys that mean something different on the swimlane screen. Returns true when the
    /// key was used.
    fn lanes_key(&mut self, key: KeyEvent, s: &Session, session: &SessionId, now: Time) -> bool {
        let (w, playhead) = self.lanes.window(s, now);
        let fit = lanes::fit_secs(s, now);
        let width = self.lanes.width.max(1);
        let col =
            chrono::Duration::milliseconds((w.end - w.start).num_milliseconds() / width as i64);
        let st = &mut self.lanes;
        // Leaves follow mode with the window and playhead where they are now.
        let unfollow = |st: &mut LanesState| {
            if st.follow {
                st.follow = false;
                st.end = w.end;
                st.playhead = playhead;
            }
        };
        let set_playhead = |st: &mut LanesState, t: Time| {
            let t = t.min(now);
            st.playhead = t;
            let secs = st.span.secs(fit);
            if st.span == Span::Fit {
                return;
            }
            // Keep the playhead inside the window by sliding the window.
            if t < st.end - chrono::Duration::seconds(secs) {
                st.end = t + chrono::Duration::seconds(secs / 10);
            } else if t > st.end {
                st.end = (t + chrono::Duration::seconds(secs / 10)).min(now);
            }
        };
        match key.code {
            KeyCode::Left | KeyCode::Right => {
                unfollow(st);
                let p = if key.code == KeyCode::Left {
                    st.playhead - col
                } else {
                    st.playhead + col
                };
                set_playhead(st, p);
            }
            KeyCode::Char(',') | KeyCode::Char('.') => {
                unfollow(st);
                if let Some(t) =
                    lanes::neighbor_call(s, st.playhead, key.code == KeyCode::Char('.'))
                {
                    set_playhead(st, t);
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') | KeyCode::Char('_') => {
                let zoom_in = matches!(key.code, KeyCode::Char('+') | KeyCode::Char('='));
                let next = if zoom_in {
                    st.span.zoom_in(fit)
                } else {
                    st.span.zoom_out(fit)
                };
                if !st.follow {
                    // Keep the playhead centred where the new window allows.
                    st.end = (st.playhead + chrono::Duration::seconds(next.secs(fit) / 2)).min(now);
                }
                st.span = next;
            }
            KeyCode::Char('f') | KeyCode::Char('F') => st.span = Span::Fit,
            KeyCode::Char('0') | KeyCode::End => {
                st.follow = true;
                st.end = now;
                st.playhead = now;
            }
            KeyCode::Home => {
                unfollow(st);
                st.playhead = w.start;
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                if st.span == Span::Fit {
                    return true;
                }
                unfollow(st);
                let half = chrono::Duration::seconds(st.span.secs(fit) / 2);
                if key.code == KeyCode::PageUp {
                    st.end -= half;
                    st.playhead -= half;
                } else {
                    st.end = (st.end + half).min(now);
                    st.playhead = (st.playhead + half).min(now);
                }
            }
            KeyCode::Tab => {
                st.focus = if st.focus == LanesFocus::Lanes {
                    LanesFocus::Calls
                } else {
                    LanesFocus::Lanes
                };
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                let d: isize = if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    -1
                } else {
                    1
                };
                match st.focus {
                    LanesFocus::Lanes => {
                        let n = lanes::lane_rows(s, &w, width, now).len();
                        st.lane_index = step(st.lane_index, d, n);
                    }
                    LanesFocus::Calls => {
                        let n = view::call_lines_before(s, CALL_LIMIT, Some(playhead)).len();
                        st.call_index = step(st.call_index, d, n);
                    }
                }
            }
            KeyCode::Enter => match st.focus {
                LanesFocus::Lanes => {
                    if let Some(row) = lanes::lane_rows(s, &w, width, now).get(st.lane_index) {
                        let agent = row.agent.clone();
                        self.push(Screen::Agent {
                            session: session.clone(),
                            agent,
                        });
                    }
                }
                LanesFocus::Calls => {
                    if let Some(c) =
                        view::call_lines_before(s, CALL_LIMIT, Some(playhead)).get(st.call_index)
                    {
                        let (agent, call) = (c.agent.clone(), c.call.clone());
                        self.push(Screen::Call {
                            session: session.clone(),
                            agent,
                            call,
                        });
                    }
                }
            },
            _ => return false,
        }
        true
    }
}

/// The window the swimlane screen shows, exported for rendering.
pub fn lanes_window(app: &App, s: &Session, now: Time) -> (Window, Time) {
    app.lanes.window(s, now)
}
