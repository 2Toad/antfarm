//! The swimlane screen: one lane per agent over a time window, a bar per call colored by
//! tool family, a playhead, and what the session looked like at the playhead.

use antfarm_core::{ResultStatus, SessionId, Snapshot, Time, ToolCategory};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell as TCell, Paragraph, Row, Table, TableState};

use crate::tui::app::{App, CALL_LIMIT, LanesFocus, lanes_window};
use crate::tui::theme::Theme;
use crate::tui::widgets;
use antfarm_view::fmt::{clock, truncate};
use antfarm_view::lanes::{self, Cell, StateAt};
use antfarm_view::view::{self, Class};

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
    let label_w: usize = if area.width >= 80 { 22 } else { 14 };
    // The lanes sit inside a bordered box: two columns of border, the label, a space.
    let bar_w = (area.width as usize).saturating_sub(2 + label_w + 1).max(1);
    app.lanes.width = bar_w;
    let (w, playhead) = lanes_window(app, s, now);
    let rows = lanes::lane_rows(s, &w, bar_w, now);
    app.lanes.lane_index = app.lanes.lane_index.min(rows.len().saturating_sub(1));
    let also = widgets::waiting_line(snap, app, None, now, t);
    let panels = area.height >= 24;
    let lanes_h = (rows.len() as u16 + 2).min(
        area.height
            .saturating_sub(if panels { 14 } else { 6 })
            .max(3),
    );
    let [
        head,
        also_a,
        controls,
        axis_a,
        lanes_a,
        legend,
        panel_a,
        keys,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(also.is_some())),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(lanes_h),
        Constraint::Length(1),
        Constraint::Min(if panels { 4 } else { 0 }),
        Constraint::Length(1),
    ])
    .areas(area);
    widgets::header(f, head, app, snap, now, t);
    if let Some(l) = also {
        f.render_widget(Paragraph::new(l), also_a);
    }

    let floor = lanes::history_start(s);
    let mut ctl = vec![
        Span::styled(" span ", t.label()),
        Span::styled(app.lanes.span.label(), t.strong()),
        Span::styled(format!("   {}", window_label(&w)), t.dim()),
        Span::styled("   playhead ", t.label()),
        Span::styled(clock(playhead), t.strong()),
        Span::styled(
            if app.lanes.follow {
                "   following now"
            } else {
                "   paused · End jumps to now"
            },
            t.dim(),
        ),
    ];
    if let Some(note) = lanes::history_note(s) {
        ctl.push(Span::styled(format!("   {note}"), t.dim()));
    } else if let Some(h) = floor.filter(|h| *h > w.start) {
        ctl.push(Span::styled(
            format!("   session starts {}", clock(h)),
            t.dim(),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(ctl)), controls);

    // Axis: tick labels over the bar area.
    let mut axis = vec![' '; bar_w];
    // A label that would run past the right edge is left out rather than clipped.
    for (col, label) in lanes::axis(&w, bar_w)
        .into_iter()
        .filter(|(c, l)| c + l.chars().count() <= bar_w)
    {
        for (i, ch) in label.chars().enumerate() {
            if let Some(slot) = axis.get_mut(col + i) {
                *slot = ch;
            }
        }
    }
    let ph_col = lanes::col_of(playhead, &w, bar_w);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" ".repeat(label_w + 2)),
            Span::styled(axis.into_iter().collect::<String>(), t.dim()),
        ])),
        axis_a,
    );

    let focus_lanes = app.lanes.focus == LanesFocus::Lanes;
    let block = Block::bordered().title(Span::styled(
        format!(" AGENTS · {} lanes ", rows.len()),
        if focus_lanes { t.strong() } else { t.label() },
    ));
    let inner = block.inner(lanes_a);
    f.render_widget(block, lanes_a);
    let visible = inner.height as usize;
    // Scroll only when the selection leaves the visible lanes.
    let sel = app.lanes.lane_index;
    if sel < app.lanes.lane_scroll {
        app.lanes.lane_scroll = sel;
    } else if visible > 0 && sel >= app.lanes.lane_scroll + visible {
        app.lanes.lane_scroll = sel + 1 - visible;
    }
    app.lanes.lane_scroll = app
        .lanes
        .lane_scroll
        .min(rows.len().saturating_sub(visible.max(1)));
    let first = app.lanes.lane_scroll;
    let inner_bar = (inner.width as usize).saturating_sub(label_w + 1);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
        .map(|(i, r)| {
            let indent = if r.depth > 0 {
                format!("{}└ ", "  ".repeat(r.depth.saturating_sub(1) as usize))
            } else {
                String::new()
            };
            let mut label_style = t.class(r.class);
            if i == app.lanes.lane_index && focus_lanes {
                label_style = label_style.add_modifier(Modifier::REVERSED);
            }
            let mut spans = vec![Span::styled(
                format!(
                    "{:<label_w$} ",
                    truncate(&format!("{indent}{}", r.label), label_w)
                ),
                label_style,
            )];
            for (c, cell) in r.cells.iter().enumerate().take(inner_bar) {
                let (ch, mut style) = match cell {
                    Cell::Empty => (" ", Default::default()),
                    Cell::Life(_) => ("─", t.dim()),
                    Cell::Call {
                        status: ResultStatus::Error,
                        ..
                    } => ("✕", t.error()),
                    Cell::Call { waiting: true, .. } => ("▓", t.class(Class::Waiting)),
                    Cell::Call {
                        category,
                        status: ResultStatus::Pending,
                        ..
                    } => ("▓", t.category(*category)),
                    Cell::Call { category, .. } => ("█", t.category(*category)),
                };
                let ch = if r.spawn_col == Some(c) && matches!(cell, Cell::Empty | Cell::Life(_)) {
                    "▸"
                } else {
                    ch
                };
                if ph_col == Some(c) {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(
                    if ph_col == Some(c) && ch == " " {
                        "│"
                    } else {
                        ch
                    },
                    style,
                ));
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);

    let legend_items: &[(&str, ToolCategory)] = &[
        ("shell", ToolCategory::Shell),
        ("read", ToolCategory::Read),
        ("edit", ToolCategory::Edit),
        ("search", ToolCategory::Search),
        ("web", ToolCategory::Web),
        ("spawn", ToolCategory::Spawn),
        ("ask", ToolCategory::Ask),
    ];
    let mut leg = vec![Span::raw(" ")];
    for (name, cat) in legend_items {
        leg.push(Span::styled("█", t.category(*cat)));
        leg.push(Span::raw(format!(" {name}  ")));
    }
    leg.push(Span::styled("✕", t.error()));
    leg.push(Span::raw(" failed  "));
    leg.push(Span::styled("▓", t.dim()));
    leg.push(Span::raw(" running  ─ alive  ▸ spawned"));
    f.render_widget(Paragraph::new(Line::from(leg)), legend);

    if panels {
        let [agents_a, calls_a] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(panel_a);
        let at = lanes::agents_at(s, playhead, now);
        let alines: Vec<Line> = at
            .iter()
            .map(|a| {
                let indent = if a.depth > 0 {
                    format!("{}└ ", "  ".repeat(a.depth.saturating_sub(1) as usize))
                } else {
                    String::new()
                };
                let class = match a.state {
                    StateAt::Finished => Class::Finished,
                    StateAt::WaitingOnAgents | StateAt::Running => Class::Working,
                    StateAt::Still => Class::Still,
                    StateAt::NotStarted => Class::Idle,
                };
                Line::from(vec![
                    Span::styled(format!("{} ", class.glyph()), t.class(class)),
                    Span::raw(format!(
                        "{:<24}",
                        truncate(&format!("{indent}{}", a.label), 24)
                    )),
                    Span::styled(a.state.label(), t.class(class)),
                ])
            })
            .collect();
        f.render_widget(
            Paragraph::new(alines).block(Block::bordered().title(Span::styled(
                format!(" AGENTS AT {} ", clock(playhead)),
                t.label(),
            ))),
            agents_a,
        );
        let calls = view::call_lines_before(s, CALL_LIMIT, Some(playhead));
        let focus_calls = app.lanes.focus == LanesFocus::Calls;
        let crow: Vec<Row> = calls
            .iter()
            .map(|c| {
                // Rewound: a call that had not returned yet at the playhead was running then.
                let done_by_playhead = match (c.at, c.elapsed) {
                    (Some(a), Some(ms)) => a + chrono::Duration::milliseconds(ms) <= playhead,
                    // No finish time recorded: trust the final status.
                    _ => true,
                };
                let result = match c.status {
                    _ if !done_by_playhead && c.status != ResultStatus::Pending => {
                        Span::styled("…", t.class(Class::Working))
                    }
                    ResultStatus::Error => Span::styled("✕", t.error()),
                    ResultStatus::Pending => Span::styled("…", t.class(Class::Working)),
                    ResultStatus::Ok => Span::raw(view::ms(c.elapsed)),
                };
                Row::new(vec![
                    TCell::from(Span::styled(c.at.map(clock).unwrap_or_default(), t.dim())),
                    TCell::from(truncate(&c.agent_name, 16)),
                    TCell::from(Span::styled(
                        c.tool.clone(),
                        t.category(ToolCategory::of(&c.tool)),
                    )),
                    TCell::from(c.target.clone()),
                    TCell::from(result),
                ])
            })
            .collect();
        let table = Table::new(
            crow,
            [
                Constraint::Length(8),
                Constraint::Length(16),
                Constraint::Length(9),
                Constraint::Min(10),
                Constraint::Length(7),
            ],
        )
        .block(Block::bordered().title(Span::styled(
            format!(" CALLS UP TO {} · NEWEST FIRST ", clock(playhead)),
            if focus_calls { t.strong() } else { t.label() },
        )))
        .row_highlight_style(if focus_calls {
            t.selected()
        } else {
            Default::default()
        });
        let mut state = TableState::default().with_selected(if calls.is_empty() {
            None
        } else {
            Some(app.lanes.call_index.min(calls.len() - 1))
        });
        f.render_stateful_widget(table, calls_a, &mut state);
    }
    widgets::key_bar(f, keys, app, now, t);
}

/// `HH:MM:SS → HH:MM:SS`, with dates and the length when the window crosses midnight.
fn window_label(w: &lanes::Window) -> String {
    if w.start.date_naive() == w.end.date_naive() && w.secs() < 86_400 {
        format!("{} → {}", clock(w.start), clock(w.end))
    } else {
        format!(
            "{} {} → {} {}  ({})",
            antfarm_view::fmt::date(w.start),
            antfarm_view::fmt::clock_hm(w.start),
            antfarm_view::fmt::date(w.end),
            antfarm_view::fmt::clock_hm(w.end),
            antfarm_view::fmt::dur(w.secs())
        )
    }
}
