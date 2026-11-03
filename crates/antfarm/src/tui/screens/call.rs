//! The tool call screen: what was asked, when, how it ended, and the input with secrets
//! masked.

use antfarm_core::{AgentId, ResultStatus, SessionId, Snapshot, Time, ToolCallId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::fmt::dur;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::{self, Class};
use crate::tui::widgets;

#[allow(clippy::too_many_arguments)]
pub fn render(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    snap: &Snapshot,
    sid: &SessionId,
    aid: &AgentId,
    cid: &ToolCallId,
    now: Time,
    t: &Theme,
) {
    let detail = snap
        .machine
        .sessions
        .get(sid)
        .and_then(|s| s.agents.get(aid))
        .and_then(|a| view::call_detail(a, cid, now));
    let Some(d) = detail else {
        f.render_widget(
            Paragraph::new(" This call is no longer in the agent's recent history. Esc goes back."),
            area,
        );
        return;
    };
    let [head, info, input, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    widgets::header(f, head, app, snap, now, t);
    let (state, style) = match d.status {
        ResultStatus::Pending => ("RUNNING", t.class(Class::Working)),
        ResultStatus::Ok => ("OK", t.ok()),
        ResultStatus::Error => ("ERROR", t.error()),
    };
    let time = |x: Option<Time>| x.map(crate::fmt::clock).unwrap_or_else(|| "-".into());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!(" {state} "), style.patch(t.strong())),
            Span::styled(format!("  {}", d.tool), t.strong()),
            Span::styled(
                format!(
                    "  {}  {:?}",
                    if d.background {
                        "background"
                    } else {
                        "foreground"
                    },
                    d.category
                )
                .to_lowercase(),
                t.dim(),
            ),
            Span::styled(
                format!("  {}", d.elapsed.map(dur).unwrap_or_default()),
                style,
            ),
        ]),
        Line::from(vec![
            Span::styled("target   ", t.label()),
            Span::raw(d.summary.clone()),
        ]),
        Line::from(vec![
            Span::styled("started  ", t.label()),
            Span::raw(time(d.started)),
            Span::styled("   finished ", t.label()),
            Span::raw(time(d.finished)),
        ]),
    ];
    if d.status == ResultStatus::Pending
        && !d.background
        && d.category == antfarm_core::ToolCategory::Shell
    {
        lines.push(Line::from(Span::styled(
            "running · no output until the call returns",
            t.class(Class::Working),
        )));
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(Span::styled(" CALL ", t.label()))),
        info,
    );
    let body = d
        .input
        .unwrap_or_else(|| "The input is not kept for this call.".into());
    f.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0))
            .block(Block::bordered().title(Span::styled(" INPUT · secrets masked ", t.label()))),
        input,
    );
    widgets::key_bar(f, keys, app, now, t);
}
