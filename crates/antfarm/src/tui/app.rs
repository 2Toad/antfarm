//! UI state and key handling. Everything that changes the screen goes through
//! [`App::handle_key`]; side effects come back as [`Action`]s for the run loop.

use antfarm_core::{AgentId, SessionId, Snapshot, Time, ToolCallId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::view::{self, Prefs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Machine,
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
        match key.code {
            KeyCode::Char('q') => self.quit = true,
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
            Screen::Call { .. } => {}
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
