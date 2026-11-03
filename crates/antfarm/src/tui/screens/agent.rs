//! The agent screen: its brief and a call waterfall.

use antfarm_core::{AgentId, ResultStatus, SessionId, Snapshot, Time};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState, Wrap};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::{self, gauge};
use antfarm_view::fmt::dur;
use antfarm_view::view::{self, Class};

#[allow(clippy::too_many_arguments)]
pub fn render(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    snap: &Snapshot,
    sid: &SessionId,
    aid: &AgentId,
    now: Time,
    t: &Theme,
) {
    let Some((s, a)) = snap
        .machine
        .sessions
        .get(sid)
        .and_then(|s| s.agents.get(aid).map(|a| (s, a)))
    else {
        f.render_widget(Paragraph::new(" This agent is gone. Esc goes back."), area);
        return;
    };
    let [head, info, list, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    widgets::header(f, head, app, snap, now, t);
    let class = Class::of_agent(a.status);
    let name = if a.parent.is_none() {
        view::session_name(s)
    } else {
        view::agent_name(a)
    };
    let elapsed = a
        .started
        .map(|st| dur((a.finished.unwrap_or(now) - st).num_seconds()))
        .unwrap_or_else(|| "-".into());
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} {name}", class.glyph()),
                t.class(class).patch(t.strong()),
            ),
            Span::styled(
                format!(
                    "   {} · {} · depth {} · {elapsed}",
                    a.kind,
                    if a.background {
                        "background"
                    } else {
                        "foreground"
                    },
                    a.depth
                ),
                t.dim(),
            ),
        ]),
        Line::from(vec![
            Span::styled("model    ", t.label()),
            Span::raw(format!(
                "{} {}",
                a.model.as_deref().unwrap_or("-"),
                a.effort
                    .as_deref()
                    .map(|e| format!("· {e}"))
                    .unwrap_or_default()
            )),
            Span::styled("   cost ", t.label()),
            Span::raw(
                a.cost_usd
                    .map(|c| format!("${c:.2}"))
                    .unwrap_or_else(|| "-".into()),
            ),
            Span::styled("   context ", t.label()),
            Span::raw(gauge(a.context.pct, 10)),
        ]),
        Line::from(vec![
            Span::styled("brief    ", t.label()),
            Span::raw(
                a.description
                    .clone()
                    .unwrap_or_else(|| s.task.clone().unwrap_or_else(|| "-".into())),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::bordered().title(Span::styled(" AGENT ", t.label()))),
        info,
    );

    let water = view::waterfall(a);
    let rows: Vec<Row> = water
        .iter()
        .map(|w| {
            let n = if w.count > 1 {
                format!("{}-{}", w.first, w.first + w.count - 1)
            } else {
                w.first.to_string()
            };
            let tool = if w.count > 1 {
                format!("{} ×{}", w.tool, w.count)
            } else {
                w.tool.clone()
            };
            let (shape, style) = match w.status {
                ResultStatus::Ok => ("ok", t.ok()),
                ResultStatus::Error => ("✕", t.error()),
                ResultStatus::Pending => ("…", t.class(Class::Working)),
            };
            Row::new(vec![
                Cell::from(Span::styled(n, t.dim())),
                Cell::from(Span::styled(
                    w.at.map(antfarm_view::fmt::clock).unwrap_or_default(),
                    t.dim(),
                )),
                Cell::from(tool),
                Cell::from(w.target.clone()),
                Cell::from(Span::styled(shape, style)),
                Cell::from(view::ms(w.elapsed_ms)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Min(20),
            Constraint::Length(2),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(["#", "TIME", "TOOL", "TARGET", "", "DUR"]).style(t.strong()))
    .block(Block::bordered().title(Span::styled(format!(" CALLS · {} ", a.calls), t.label())))
    .row_highlight_style(t.selected());
    let mut state = TableState::default().with_selected(if water.is_empty() {
        None
    } else {
        Some(app.water_index.min(water.len() - 1))
    });
    f.render_stateful_widget(table, list, &mut state);
    widgets::key_bar(f, keys, app, now, t);
}
