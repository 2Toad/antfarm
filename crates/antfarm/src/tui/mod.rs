//! The interactive terminal UI.

mod app;
mod screens;
mod theme;
pub use antfarm_view::view;
mod widgets;

use std::time::Duration;

use antfarm_core::{Snapshot, Time};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use serde_json::Value;

use app::{Action, App, Screen};
use theme::Theme;

pub async fn run() -> anyhow::Result<()> {
    let live = crate::cmd::start_live();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &live).await;
    ratatui::restore();
    live.handle.stop();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    live: &crate::cmd::Live,
) -> anyhow::Result<()> {
    let mut app = App::default();
    let theme = Theme::from_env();
    let mut events = EventStream::new();
    let mut snapshots = live.handle.snapshots();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let panic_test = std::env::var_os("ANTFARM_PANIC_TEST").is_some();
    loop {
        let snap = snapshots.borrow_and_update().clone();
        let now = chrono::Utc::now();
        if let Screen::Agent { session, agent } | Screen::Call { session, agent, .. } = app.screen()
        {
            live.wants.want(session, agent);
        }
        terminal.draw(|f| draw(f, &mut app, &snap, now, &theme))?;
        if panic_test {
            panic!("ANTFARM_PANIC_TEST is set");
        }
        if app.quit {
            return Ok(());
        }
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => {
                    if let Some(action) = app.handle_key(k, &snap, now) {
                        perform(&mut app, action, &snap, now);
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            r = snapshots.changed() => {
                if r.is_err() {
                    return Ok(());
                }
            }
            _ = tick.tick() => {}
        }
    }
}

fn perform(app: &mut App, action: Action, snap: &Snapshot, now: Time) {
    let msg = match &action {
        Action::GoTo(id) => match snap.machine.sessions.get(id) {
            Some(s) => {
                antfarm_core::actions::go(s).unwrap_or_else(|e| format!("can't switch: {e}"))
            }
            None => "that session is gone".into(),
        },
        Action::Copy(id) => match copy_text(app, snap, id, now) {
            Some(text) => {
                antfarm_core::actions::copy(&text).unwrap_or_else(|e| format!("can't copy: {e}"))
            }
            None => "nothing to copy here".into(),
        },
    };
    app.toast(msg, now);
}

/// What C copies: the command or question of the call on screen, else the one the
/// session is waiting on or busy with.
fn copy_text(
    app: &App,
    snap: &Snapshot,
    id: &antfarm_core::SessionId,
    now: Time,
) -> Option<String> {
    let s = snap.machine.sessions.get(id)?;
    if let Screen::Call { agent, call, .. } = app.screen() {
        let a = s.agents.get(agent)?;
        let input = a.activity.iter().rev().find_map(|x| match x {
            antfarm_core::Activity::Call {
                call: c,
                input,
                summary,
                ..
            } if c == call => Some((input.clone(), summary.clone())),
            _ => None,
        })?;
        return Some(input.0.as_deref().and_then(command_of).unwrap_or(input.1));
    }
    if let Some(w) = view::waiting(s, now) {
        return Some(w.request);
    }
    let c = view::blocking_call(s).or_else(|| s.open_calls().max_by_key(|c| c.started))?;
    Some(command_of(&c.input).unwrap_or_else(|| c.summary.clone()))
}

fn command_of(v: &Value) -> Option<String> {
    v.get("command").and_then(Value::as_str).map(str::to_owned)
}

pub fn draw(f: &mut Frame, app: &mut App, snap: &Snapshot, now: Time, t: &Theme) {
    let area = f.area();
    if area.width < 40 || area.height < 10 {
        widgets::too_small(f, area);
        return;
    }
    match app.screen().clone() {
        Screen::Machine => screens::machine::render(f, area, app, snap, now, t),
        Screen::Lanes { session } => screens::lanes::render(f, area, app, snap, &session, now, t),
        Screen::Session { session } => {
            screens::session::render(f, area, app, snap, &session, now, t)
        }
        Screen::Agent { session, agent } => {
            screens::agent::render(f, area, app, snap, &session, &agent, now, t)
        }
        Screen::Call {
            session,
            agent,
            call,
        } => screens::call::render(f, area, app, snap, &session, &agent, &call, now, t),
    }
    if app.help {
        help(f, area, t);
    }
}

fn help(f: &mut Frame, area: Rect, t: &Theme) {
    let keys: &[(&str, &str)] = &[
        ("Enter", "drill in: session, agent, call"),
        ("Esc", "back out, or clear the filter"),
        ("↑ ↓ j k PgUp PgDn", "move"),
        ("Tab", "session screen: switch between agents and calls"),
        ("/", "filter: words, state:waiting, project:api"),
        ("S  F6", "sort: needs-you, cost, name"),
        ("T  F5", "show or hide sub-agents"),
        (
            "L",
            "swimlanes: ←→ playhead, , . step calls, + - zoom, F fit, End now",
        ),
        ("f", "show sub-agents that finished recently"),
        ("a", "show exited sessions"),
        ("N", "next session waiting for you"),
        ("G", "go to the session's terminal or editor"),
        ("C", "copy the command"),
        ("Z", "snooze a waiting session for 10 minutes"),
        ("M", "mute a session: dimmed and sorted last"),
        ("q  Ctrl-C", "quit"),
    ];
    let h = keys.len() as u16 + 4;
    let w = 72.min(area.width);
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(h),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, rect, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(w),
        Constraint::Fill(1),
    ])
    .areas(mid);
    let mut lines: Vec<Line> = keys
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!(" {k:<20}"), t.strong()),
                Span::raw(*d),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " antfarm only watches. You answer in the terminal.",
        t.dim(),
    )));
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" keys · any key closes ")),
        rect,
    );
}

#[cfg(test)]
mod tests;
