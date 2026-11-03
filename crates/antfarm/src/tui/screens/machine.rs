//! The machine screen: meters, the needs-you box, the session table, and a detail box
//! for the selected session.

use antfarm_core::{Snapshot, Time, WaitingKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState};

use crate::fmt::{dur, money, truncate};
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::{self, Class};
use crate::tui::widgets::{self, bar, gauge, rate, sparkline};

pub fn render(f: &mut Frame, area: Rect, app: &mut App, snap: &Snapshot, now: Time, t: &Theme) {
    let rows = view::table_rows(snap, &app.prefs, app.show_exited, now);
    app.sync_machine(&rows);
    let needs = view::needs_you(snap, &app.prefs, now);
    let needs_h = if needs.is_empty() {
        0
    } else {
        needs.len().min(4) as u16 + 2
    };
    let detail_h = if area.height >= 30 && area.width >= 80 {
        7
    } else {
        0
    };
    let [head, meters, needs_a, table_a, detail_a, status_a, keys_a] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(needs_h),
        Constraint::Min(3),
        Constraint::Length(detail_h),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    widgets::header(f, head, app, snap, now, t);
    render_meters(f, meters, snap, now, t);
    if needs_h > 0 {
        render_needs(f, needs_a, &needs, t);
    }
    render_table(f, table_a, app, &rows, t);
    if detail_h > 0
        && let Some(r) = rows.get(app.machine_index)
        && let Some(s) = snap.machine.sessions.get(&r.session)
    {
        render_detail(f, detail_a, s, now, t);
    }
    let filter = if app.editing_filter {
        format!("{}▏", app.prefs.filter)
    } else if app.prefs.filter.is_empty() {
        "none".into()
    } else {
        app.prefs.filter.clone()
    };
    let selected = rows
        .get(app.machine_index)
        .map(|r| r.name.clone())
        .unwrap_or_default();
    let status = Line::from(vec![
        Span::styled(" filter ", t.label()),
        Span::styled(
            filter,
            if app.editing_filter {
                t.strong()
            } else {
                t.dim()
            },
        ),
        Span::styled("   sort ", t.label()),
        Span::raw(app.prefs.sort.label()),
        Span::styled("   tree ", t.label()),
        Span::raw(if app.prefs.tree { "on" } else { "off" }),
        Span::styled(format!("   rows {}", rows.len()), t.label()),
        Span::styled("   selected ", t.label()),
        Span::styled(
            selected,
            t.class(
                rows.get(app.machine_index)
                    .map(|r| r.class)
                    .unwrap_or(Class::Idle),
            ),
        ),
    ]);
    f.render_widget(Paragraph::new(status), status_a);
    widgets::key_bar(f, keys_a, app, now, t);
}

fn render_meters(f: &mut Frame, area: Rect, snap: &Snapshot, now: Time, t: &Theme) {
    let m = view::meters(snap, now);
    let total = m.agents.max(1);
    let half =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    let bw = (half[0].width as usize).saturating_sub(30).clamp(4, 30);
    let left = vec![
        Line::from(vec![
            Span::styled(" WAIT  ", t.class(Class::Waiting)),
            Span::styled(
                bar(m.waiting, m.sessions.max(1), bw),
                t.class(Class::Waiting),
            ),
            Span::styled(
                format!(
                    " {}{}",
                    m.waiting,
                    m.oldest_wait
                        .map(|w| format!(" · oldest {}", dur(w)))
                        .unwrap_or_default()
                ),
                t.class(Class::Waiting),
            ),
        ]),
        Line::from(vec![
            Span::styled(" WORK  ", t.class(Class::Working)),
            Span::styled(bar(m.working, total, bw), t.class(Class::Working)),
            Span::raw(format!(" {} agents", m.working)),
        ]),
        Line::from(vec![
            Span::styled(" DONE  ", t.class(Class::Finished)),
            Span::styled(bar(m.finished, total, bw), t.class(Class::Finished)),
            Span::raw(format!(" {} agents", m.finished)),
        ]),
    ];
    let right = vec![
        Line::from(vec![
            Span::styled("CALLS/MIN ", t.label()),
            Span::raw(format!("{:.1}", m.calls_per_min)),
            Span::styled("    TOK/MIN ", t.label()),
            Span::raw(rate(m.tok_per_min)),
        ]),
        Line::from(vec![
            Span::styled("COST      ", t.label()),
            Span::raw(format!("${:.2} across live sessions", m.spent_today)),
            Span::styled(
                format!("    still {} · exited {}", m.still, m.exited),
                t.dim(),
            ),
        ]),
        Line::from(vec![
            Span::styled("WATCHING  ", t.label()),
            Span::raw(format!("{} sessions · {} agents", m.sessions, m.agents)),
            Span::styled(
                if m.apps.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", m.apps.join(" "))
                },
                t.dim(),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(left), half[0]);
    f.render_widget(Paragraph::new(right), half[1]);
}

fn render_needs(f: &mut Frame, area: Rect, needs: &[view::NeedsYou], t: &Theme) {
    let block = Block::bordered()
        .title(Span::styled(
            format!(" NEEDS YOU · {} ", needs.len()),
            t.class(Class::Waiting),
        ))
        .border_style(t.class(Class::Waiting));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;
    let lines: Vec<Line> = needs
        .iter()
        .take(4)
        .enumerate()
        .map(|(i, n)| {
            let kind = match n.kind {
                WaitingKind::Permission => "PERMISSION",
                WaitingKind::Question => "QUESTION",
            };
            let age = n.age.map(dur).unwrap_or_default();
            let tail = format!("  {}  {age}  G term", n.source);
            let head = format!(
                "◆ {:<16} {:<14} {kind:<10} ",
                truncate(&n.name, 16),
                truncate(&n.project, 14)
            );
            let req_w = w.saturating_sub(head.chars().count() + tail.chars().count());
            let style = if i == 0 {
                t.waiting_band()
            } else {
                t.class(Class::Waiting)
            };
            Line::from(vec![
                Span::styled(head, style),
                Span::styled(format!("{:<req_w$}", truncate(&n.request, req_w)), style),
                Span::styled(tail, style),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_table(f: &mut Frame, area: Rect, app: &App, rows: &[view::Row], t: &Theme) {
    let w = area.width;
    // Columns drop as the terminal narrows: activity first, then pid, source, and
    // tokens per minute, then everything but the essentials.
    let wide = w >= 130;
    let medium = w >= 100;
    let narrow = w < 80;
    let mut header = vec!["S"];
    let mut widths = vec![Constraint::Length(1)];
    if medium {
        header.push("PID");
        widths.push(Constraint::Length(6));
    }
    header.extend(["AGENT", "PROJECT"]);
    widths.extend([
        Constraint::Length(if medium { 22 } else { 18 }),
        Constraint::Length(if medium { 14 } else { 10 }),
    ]);
    if medium {
        header.push("SOURCE");
        widths.push(Constraint::Length(10));
    }
    header.extend(["STATE", "FOR", "CURRENT / LAST CALL", "CALLS"]);
    widths.extend([
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Min(16),
        Constraint::Length(5),
    ]);
    if medium {
        header.push("TOK/M");
        widths.push(Constraint::Length(5));
    }
    header.push("COST");
    widths.push(Constraint::Length(8));
    if medium {
        header.push("CTX");
        widths.push(Constraint::Length(4));
    }
    if wide {
        header.push("ACT 10m");
        widths.push(Constraint::Length(10));
    }
    if narrow {
        header = vec!["S", "AGENT", "STATE", "FOR", "CURRENT / LAST CALL"];
        widths = vec![
            Constraint::Length(1),
            Constraint::Length(20),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Min(10),
        ];
    }
    let body: Vec<Row> = rows
        .iter()
        .map(|r| {
            let style = if r.muted { t.dim() } else { t.class(r.class) };
            let indent = if r.depth > 0 {
                format!("{}└ ", "  ".repeat(r.depth.saturating_sub(1) as usize))
            } else {
                String::new()
            };
            let name = Cell::from(Span::styled(
                format!("{indent}{}", r.name),
                if r.agent.is_none() && !r.muted {
                    t.strong()
                } else {
                    Default::default()
                },
            ));
            let call = format!("{} {}", r.tool, r.target);
            let mut cells: Vec<Cell> = vec![Cell::from(Span::styled(r.class.glyph(), style))];
            if narrow {
                cells.extend([
                    name,
                    Cell::from(Span::styled(r.state, style)),
                    Cell::from(r.for_secs.map(dur).unwrap_or_default()),
                    Cell::from(call),
                ]);
                return Row::new(cells);
            }
            if medium {
                cells.push(Cell::from(r.pid.map(|p| p.to_string()).unwrap_or_default()));
            }
            cells.push(name);
            cells.push(Cell::from(r.project.clone()));
            if medium {
                cells.push(Cell::from(r.source.clone()));
            }
            cells.push(Cell::from(Span::styled(r.state, style)));
            cells.push(Cell::from(Span::styled(
                r.for_secs.map(dur).unwrap_or_default(),
                style,
            )));
            cells.push(Cell::from(call));
            cells.push(Cell::from(r.calls.to_string()));
            if medium {
                cells.push(Cell::from(
                    r.tok_per_min
                        .filter(|v| *v > 0.0)
                        .map(rate)
                        .unwrap_or_else(|| "-".into()),
                ));
            }
            cells.push(Cell::from(r.cost.clone()));
            if medium {
                cells.push(Cell::from(
                    r.ctx
                        .map(|c| format!("{c:.0}%"))
                        .unwrap_or_else(|| "-".into()),
                ));
            }
            if wide {
                cells.push(Cell::from(Span::styled(sparkline(&r.spark), style)));
            }
            Row::new(cells)
        })
        .collect();
    let table = Table::new(body, widths)
        .header(Row::new(header).style(t.strong()))
        .row_highlight_style(t.selected())
        .column_spacing(1);
    let mut state = TableState::default().with_selected(if rows.is_empty() {
        None
    } else {
        Some(app.machine_index)
    });
    f.render_stateful_widget(table, area, &mut state);
    if rows.is_empty() {
        let msg = if app.prefs.filter.is_empty() {
            "No Claude Code sessions are running."
        } else {
            "No sessions match the filter. Esc clears it."
        };
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {msg}"), t.dim())),
            Rect {
                y: area.y + 1,
                height: 1,
                ..area
            },
        );
    }
}

fn render_detail(f: &mut Frame, area: Rect, s: &antfarm_core::Session, now: Time, t: &Theme) {
    let title = format!(
        " {} — {} — {} — ⏎ to open ",
        view::session_name(s),
        crate::fmt::project_name(s),
        s.branch.as_deref().unwrap_or("-")
    );
    let block = Block::bordered().title(Span::styled(title, t.strong()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let calls: u64 = s.agents.values().map(|a| a.calls).sum();
    let mut lines = vec![Line::from(vec![
        Span::raw(format!(
            "{} · pid {} · up {} · {} agents · {} calls · ",
            view::source_label(s),
            s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            s.started
                .map(|st| dur((now - st).num_seconds()))
                .unwrap_or_else(|| "-".into()),
            s.agents.len(),
            calls
        )),
        Span::styled(money(&s.cost), t.strong()),
        Span::raw(format!(
            " (${:.2}/min) · ctx {}",
            s.cost.usd_per_min,
            gauge(s.context.pct, 10)
        )),
    ])];
    if let Some(w) = view::waiting(s, now) {
        let kind = match w.kind {
            WaitingKind::Permission => "permission",
            WaitingKind::Question => "question",
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("◆ waiting {} {kind}: ", w.age.map(dur).unwrap_or_default()),
                t.class(Class::Waiting),
            ),
            Span::styled(truncate(&w.request, 70), t.strong()),
            Span::styled(
                w.reason
                    .map(|r| format!("   reason: {}", truncate(&r, 50)))
                    .unwrap_or_default(),
                t.dim(),
            ),
        ]));
    }
    for c in view::call_lines(s, 2) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", c.at.map(crate::fmt::clock).unwrap_or_default()),
                t.dim(),
            ),
            Span::raw(format!("{} {}", c.tool, truncate(&c.target, 90))),
        ]));
    }
    let (_, lanes) = view::lanes(s, now);
    if let Some(root) = lanes.first() {
        let width = inner.width.saturating_sub(8) as usize;
        let mut spans = vec![Span::styled("time  ", t.label())];
        spans.extend(lane_spans(root, width, t));
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// A lane drawn as block characters, one style per segment.
pub fn lane_spans(lane: &view::Lane, width: usize, t: &Theme) -> Vec<Span<'static>> {
    let mut cells = vec![None; width];
    for (a, b, class) in &lane.segments {
        let (from, to) = (
            (a * width as f64) as usize,
            ((b * width as f64).ceil() as usize).min(width),
        );
        for c in cells
            .iter_mut()
            .take(to.max(from + 1).min(width))
            .skip(from)
        {
            *c = Some(*class);
        }
    }
    let mut spans = Vec::new();
    let mut i = 0;
    while i < width {
        let class = cells[i];
        let mut j = i;
        while j < width && cells[j] == class {
            j += 1;
        }
        let text = match class {
            None => " ".repeat(j - i),
            Some(Class::Waiting) => "▓".repeat(j - i),
            Some(_) => "█".repeat(j - i),
        };
        spans.push(match class {
            None => Span::raw(text),
            Some(c) => Span::styled(text, t.class(c)),
        });
        i = j;
    }
    spans
}
