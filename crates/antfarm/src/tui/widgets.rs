//! Pieces every screen shares: the header, the needs-you band, the key bar, and small
//! text drawings (bars, sparklines, gauges).

use antfarm_core::{Snapshot, Time, WaitingKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::app::{App, Screen};
use super::theme::Theme;
use super::view::{self, Class};
use crate::fmt::{dur, truncate};

/// `antfarm  machine › project › session › agent › call  N/5   spent today $x  HH:MM:SS`
pub fn header(f: &mut Frame, area: Rect, app: &App, snap: &Snapshot, now: Time, t: &Theme) {
    let mut crumbs: Vec<(String, bool)> = vec![(host(), true)];
    let level = match app.screen() {
        Screen::Machine => 1,
        Screen::Session { session } => {
            if let Some(s) = snap.machine.sessions.get(session) {
                crumbs.push((crate::fmt::project_name(s), true));
                crumbs.push((view::session_name(s), true));
            }
            3
        }
        Screen::Agent { session, agent } | Screen::Call { session, agent, .. } => {
            if let Some(s) = snap.machine.sessions.get(session) {
                crumbs.push((crate::fmt::project_name(s), true));
                crumbs.push((view::session_name(s), true));
                if let Some(a) = s.agents.get(agent) {
                    crumbs.push((
                        if a.parent.is_none() {
                            "root".into()
                        } else {
                            truncate(&view::agent_name(a), 24)
                        },
                        true,
                    ));
                }
            }
            if matches!(app.screen(), Screen::Call { .. }) {
                5
            } else {
                4
            }
        }
    };
    for (i, name) in ["project", "session", "agent", "call"].iter().enumerate() {
        if crumbs.len() < i + 2 {
            crumbs.push(((*name).to_owned(), false));
        }
    }
    if level == 5 {
        crumbs[4].1 = true;
        crumbs[4].0 = "call".into();
    }
    let mut spans = vec![Span::styled(" antfarm ", t.brand()), Span::raw("  ")];
    for (i, (name, reached)) in crumbs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" › ", t.dim()));
        }
        spans.push(Span::styled(
            name.clone(),
            if *reached { t.strong() } else { t.dim() },
        ));
    }
    spans.push(Span::styled(format!("   {level}/5"), t.dim()));
    let m = view::meters(snap, now);
    let left_w: usize = spans.iter().map(|s| s.width()).sum();
    let full = format!(
        "spent today ${:.2}  {}",
        m.spent_today,
        crate::fmt::clock(now)
    );
    let right = if left_w + full.len() < area.width as usize {
        full
    } else {
        crate::fmt::clock(now)
    };
    let pad = (area.width as usize).saturating_sub(left_w + right.len() + 1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right, t.dim()));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn host() -> String {
    if cfg!(test) {
        return "machine".into();
    }
    std::env::var("ANTFARM_HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .arg("-s")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "machine".into())
}

/// One line naming the oldest waiting session other than `except`, with a count of the rest.
pub fn waiting_line(
    snap: &Snapshot,
    app: &App,
    except: Option<&antfarm_core::SessionId>,
    now: Time,
    t: &Theme,
) -> Option<Line<'static>> {
    let others: Vec<_> = view::needs_you(snap, &app.prefs, now)
        .into_iter()
        .filter(|w| Some(&w.session) != except)
        .collect();
    let first = others.first()?;
    let kind = match first.kind {
        WaitingKind::Permission => "permission",
        WaitingKind::Question => "question",
    };
    let mut spans = vec![
        Span::styled(" ◆ also waiting ", t.class(Class::Waiting)),
        Span::styled(first.name.clone(), t.strong()),
        Span::styled(
            format!(
                "  {}  {kind} · {} · {}",
                first.project,
                first.source,
                first.age.map(dur).unwrap_or_default()
            ),
            t.dim(),
        ),
    ];
    if others.len() > 1 {
        spans.push(Span::styled(
            format!("   +{} more", others.len() - 1),
            t.dim(),
        ));
    }
    spans.push(Span::styled("   N next waiting", t.dim()));
    Some(Line::from(spans))
}

/// The bottom line: a toast if one is showing, else the key hints for this screen.
pub fn key_bar(f: &mut Frame, area: Rect, app: &App, now: Time, t: &Theme) {
    if let Some(msg) = app.live_toast(now) {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(format!(" {msg}"), t.strong()))),
            area,
        );
        return;
    }
    let keys: &[(&str, &str)] = match app.screen() {
        Screen::Machine => &[
            ("⏎", "Drill"),
            ("/", "Filter"),
            ("S", "Sort"),
            ("T", "Tree"),
            ("N", "Next"),
            ("G", "Term"),
            ("C", "Copy"),
            ("Z", "Snooze"),
            ("M", "Mute"),
            ("?", "Help"),
            ("q", "Quit"),
        ],
        Screen::Session { .. } => &[
            ("⏎", "Open"),
            ("Tab", "Focus"),
            ("G", "Term"),
            ("C", "Copy"),
            ("Z", "Snooze"),
            ("N", "Next"),
            ("Esc", "Back"),
            ("?", "Help"),
            ("q", "Quit"),
        ],
        Screen::Agent { .. } => &[
            ("⏎", "Call"),
            ("G", "Term"),
            ("N", "Next"),
            ("Esc", "Back"),
            ("?", "Help"),
            ("q", "Quit"),
        ],
        Screen::Call { .. } => &[
            ("↑↓", "Scroll"),
            ("C", "Copy"),
            ("G", "Term"),
            ("Esc", "Back"),
            ("?", "Help"),
            ("q", "Quit"),
        ],
    };
    let mut spans = Vec::new();
    for (k, label) in keys {
        spans.push(Span::styled(k.to_string(), t.strong()));
        spans.push(Span::styled(format!("{label} "), t.key()));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// `[||||||    ]` scaled to `max`.
pub fn bar(value: usize, max: usize, width: usize) -> String {
    let fill = if max == 0 {
        0
    } else {
        (value * width).div_ceil(max).min(width)
    };
    format!("[{}{}]", "|".repeat(fill), " ".repeat(width - fill))
}

/// Ten values as block characters.
pub fn sparkline(v: &[u32]) -> String {
    const B: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = v.iter().copied().max().unwrap_or(0);
    v.iter()
        .map(|x| {
            if *x == 0 {
                ' '
            } else {
                B[((*x as usize * 7) / max.max(1) as usize).min(7)]
            }
        })
        .collect()
}

/// `██████░░░░ 62%`
pub fn gauge(pct: Option<f32>, width: usize) -> String {
    match pct {
        None => "-".into(),
        Some(p) => {
            let fill = ((p.clamp(0.0, 100.0) / 100.0) * width as f32).round() as usize;
            format!("{}{} {p:.0}%", "█".repeat(fill), "░".repeat(width - fill))
        }
    }
}

pub fn rate(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("{:.1}M", v / 1_000_000.0)
    } else if v >= 10_000.0 {
        format!("{:.0}k", v / 1000.0)
    } else if v >= 1000.0 {
        format!("{:.1}k", v / 1000.0)
    } else {
        format!("{v:.0}")
    }
}

pub fn too_small(f: &mut Frame, area: Rect) {
    f.render_widget(Paragraph::new("terminal too small for antfarm"), area);
}
