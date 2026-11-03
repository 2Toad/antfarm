//! The session screen: what it is waiting on, its agents and status, a timeline lane per
//! agent, and its calls newest first.

use antfarm_core::{ResultStatus, Session, SessionId, Snapshot, Time, WaitingKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState};

use super::machine::lane_spans;
use crate::fmt::{dur, money, truncate};
use crate::tui::app::{App, CALL_LIMIT, Focus};
use crate::tui::theme::Theme;
use crate::tui::view::{self, Class};
use crate::tui::widgets::{self, gauge};

pub fn render(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    snap: &Snapshot,
    id: &SessionId,
    now: Time,
    t: &Theme,
) {
    let Some(s) = snap.machine.sessions.get(id) else {
        f.render_widget(
            Paragraph::new(" This session is gone. Esc goes back."),
            area,
        );
        return;
    };
    let also = widgets::waiting_line(snap, app, Some(id), now, t);
    let wait = view::waiting(s, now);
    let agents = view::agent_lines(s, now);
    let (span, lanes) = view::lanes(s, now);
    let wait_h = if wait.is_some() { 7 } else { 0 };
    let boxes_h = (agents.len() as u16 + 2).clamp(8, 10);
    let lanes_h = if area.height >= 30 {
        lanes.len().min(6) as u16 + 2
    } else {
        0
    };
    let [head, also_a, wait_a, boxes, lanes_a, calls_a, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(also.is_some())),
        Constraint::Length(wait_h),
        Constraint::Length(boxes_h),
        Constraint::Length(lanes_h),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    widgets::header(f, head, app, snap, now, t);
    if let Some(l) = also {
        f.render_widget(Paragraph::new(l), also_a);
    }
    if let Some(w) = &wait {
        render_waiting(f, wait_a, s, w, t);
    }
    let [agents_a, status_a] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(boxes);
    render_agents(f, agents_a, app, &agents, t);
    render_status(f, status_a, s, now, t);
    if lanes_h > 0 {
        render_lanes(f, lanes_a, span, &lanes, t);
    }
    render_calls(f, calls_a, app, s, t);
    widgets::key_bar(f, keys, app, now, t);
}

fn render_waiting(f: &mut Frame, area: Rect, s: &Session, w: &view::Waiting, t: &Theme) {
    let kind = match w.kind {
        WaitingKind::Permission => "permission",
        WaitingKind::Question => "question",
    };
    let block = Block::bordered()
        .title(Span::styled(format!(" WAITING FOR YOU · {kind} "), t.class(Class::Waiting)))
        .title_top(Line::from(Span::styled(format!(" {} ", w.age.map(dur).unwrap_or_default()), t.waiting_band())).right_aligned())
        .title_bottom(Line::from(Span::styled(format!(" G go to terminal · {}   C copy   Z snooze   antfarm only watches; you answer in the terminal ", view::source_label(s)), t.dim())).right_aligned())
        .border_style(t.class(Class::Waiting));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let intro = match w.kind {
        WaitingKind::Question => format!("The {} agent asked:", w.agent),
        WaitingKind::Permission if w.tool.is_empty() => {
            format!("The {} agent is waiting:", w.agent)
        }
        WaitingKind::Permission => format!(
            "The {} agent wants to run {} and needs your OK:",
            w.agent, w.tool
        ),
    };
    let request = if w.tool.is_empty() {
        Line::from(vec![
            Span::styled("  Claude Code reports: ", t.dim()),
            Span::styled(w.request.clone(), t.strong()),
        ])
    } else {
        let lead = if w.kind == WaitingKind::Question {
            "  ? "
        } else {
            "  $ "
        };
        Line::from(vec![
            Span::styled(lead, t.dim()),
            Span::styled(
                truncate(&w.request, inner.width.saturating_sub(4) as usize),
                t.strong(),
            ),
        ])
    };
    let mut lines = vec![Line::from(intro), request];
    if let Some(r) = &w.reason {
        lines.push(Line::from(vec![
            Span::styled("reason   ", t.label()),
            Span::raw(format!("“{r}”")),
        ]));
    }
    if !w.touches.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("touches  ", t.label()),
            Span::raw(w.touches.join(" · ")),
        ]));
    }
    let asked = w.asked_at.map(crate::fmt::clock).unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled("asked    ", t.label()),
        Span::raw(match &w.after {
            Some(a) => format!("{asked}, right after {}", truncate(a, 60)),
            None => asked,
        }),
    ]));
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_agents(f: &mut Frame, area: Rect, app: &App, agents: &[view::AgentLine], t: &Theme) {
    let focused = app.focus == Focus::Agents;
    let block = Block::bordered().title(Span::styled(
        " AGENTS ",
        if focused { t.strong() } else { t.label() },
    ));
    let rows: Vec<Row> = agents
        .iter()
        .map(|a| {
            let indent = if a.depth > 0 {
                format!("{}└ ", "  ".repeat(a.depth.saturating_sub(1) as usize))
            } else {
                String::new()
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{indent}{} {}", a.class.glyph(), a.name),
                    t.class(a.class),
                )),
                Cell::from(Span::styled(a.state, t.class(a.class))),
                Cell::from(Span::styled(a.detail.clone(), t.dim())),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Min(16),
            Constraint::Length(6),
            Constraint::Length(18),
        ],
    )
    .block(block)
    .row_highlight_style(if focused {
        t.selected()
    } else {
        Default::default()
    });
    let mut state = TableState::default()
        .with_selected(Some(app.agent_index.min(agents.len().saturating_sub(1))));
    f.render_stateful_widget(table, area, &mut state);
}

fn render_status(f: &mut Frame, area: Rect, s: &Session, now: Time, t: &Theme) {
    let block = Block::bordered().title(Span::styled(" STATUS ", t.label()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let class = Class::of_session(s.status);
    let row = |k: &str, v: Vec<Span<'static>>| {
        let mut spans = vec![Span::styled(format!("{k:<9}"), t.label())];
        spans.extend(v);
        Line::from(spans)
    };
    let last_call = view::call_lines(s, 1).first().and_then(|c| c.at);
    let lines = vec![
        row(
            "state",
            vec![
                Span::styled(s.status.label().to_owned(), t.class(class)),
                Span::styled(
                    s.status_since
                        .map(|x| format!(" for {}", dur((now - x).num_seconds())))
                        .unwrap_or_default(),
                    t.dim(),
                ),
            ],
        ),
        row(
            "source",
            vec![Span::raw(format!(
                "{} · pid {} · {}",
                view::source_label(s),
                s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                s.branch.as_deref().unwrap_or("-")
            ))],
        ),
        row(
            "active",
            vec![Span::raw(format!(
                "up {} · last call {}",
                s.started
                    .map(|st| dur((now - st).num_seconds()))
                    .unwrap_or_else(|| "-".into()),
                last_call
                    .map(|l| format!("{} ago", dur((now - l).num_seconds())))
                    .unwrap_or_else(|| "-".into())
            ))],
        ),
        row(
            "tokens",
            vec![Span::raw(format!(
                "{} in · {} out · {} cache read",
                widgets::rate(s.usage.input as f64 + s.usage.cache_creation as f64),
                widgets::rate(s.usage.output as f64),
                widgets::rate(s.usage.cache_read as f64)
            ))],
        ),
        row(
            "cost",
            vec![
                Span::styled(money(&s.cost), t.strong()),
                Span::raw(format!(" so far · ${:.2}/min", s.cost.usd_per_min)),
            ],
        ),
        row("context", vec![Span::raw(gauge(s.context.pct, 14))]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_lanes(
    f: &mut Frame,
    area: Rect,
    span: Option<(Time, Time)>,
    lanes: &[view::Lane],
    t: &Theme,
) {
    let title = match span {
        Some((a, b)) => {
            format!(" TIMELINE {} → now ", crate::fmt::clock_hm(a))
                + &format!("({}) ", dur((b - a).num_seconds()))
        }
        None => " TIMELINE ".into(),
    };
    let block = Block::bordered().title(Span::styled(title, t.label()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let label_w = 20usize;
    let width = (inner.width as usize).saturating_sub(label_w + 1);
    let lines: Vec<Line> = lanes
        .iter()
        .map(|l| {
            let indent = if l.depth > 0 {
                format!("{}└ ", "  ".repeat(l.depth.saturating_sub(1) as usize))
            } else {
                String::new()
            };
            let mut spans = vec![Span::styled(
                format!(
                    "{:<label_w$} ",
                    truncate(&format!("{indent}{}", l.label), label_w)
                ),
                t.dim(),
            )];
            spans.extend(lane_spans(l, width, t));
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_calls(f: &mut Frame, area: Rect, app: &App, s: &Session, t: &Theme) {
    let calls = view::call_lines(s, CALL_LIMIT);
    let focused = app.focus == Focus::Calls;
    let rows: Vec<Row> = calls
        .iter()
        .map(|c| {
            let style = if c.waiting {
                t.class(Class::Waiting)
            } else {
                Default::default()
            };
            let result = match (c.waiting, c.status) {
                (true, _) => Span::styled("asked", t.class(Class::Waiting)),
                (_, ResultStatus::Error) => {
                    Span::styled(format!("✕ {}", view::ms(c.elapsed)), t.error())
                }
                (_, ResultStatus::Pending) => Span::styled("running", t.class(Class::Working)),
                (_, ResultStatus::Ok) => Span::raw(view::ms(c.elapsed)),
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    c.at.map(crate::fmt::clock).unwrap_or_default(),
                    t.dim(),
                )),
                Cell::from(Span::styled(truncate(&c.agent_name, 18), style)),
                Cell::from(Span::styled(c.tool.clone(), style)),
                Cell::from(Span::styled(c.target.clone(), style)),
                Cell::from(result),
            ])
            .style(style)
        })
        .collect();
    let block = Block::bordered().title(Span::styled(
        format!(
            " TOOL CALLS · NEWEST FIRST · {} ",
            if (calls.len() as u64) < view::call_count(s) {
                format!("{} of {}", calls.len(), view::call_count(s))
            } else {
                calls.len().to_string()
            }
        ),
        if focused { t.strong() } else { t.label() },
    ));
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Min(20),
            Constraint::Length(9),
        ],
    )
    .header(Row::new(["TIME", "AGENT", "TOOL", "TARGET", "RESULT"]).style(t.strong()))
    .block(block)
    .row_highlight_style(if focused {
        t.selected()
    } else {
        Default::default()
    });
    let mut state = TableState::default().with_selected(if calls.is_empty() {
        None
    } else {
        Some(app.call_index.min(calls.len() - 1))
    });
    f.render_stateful_widget(table, area, &mut state);
}
