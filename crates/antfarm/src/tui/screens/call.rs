//! The tool call screen: what was asked, when, how it ended, and the input and result
//! with secrets masked.

use antfarm_core::{AgentId, ResultStatus, SessionId, Snapshot, Time, ToolCallId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets;
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
            Paragraph::new(" This call isn't in the agent's history. Esc goes back."),
            area,
        );
        return;
    };
    let (state, style) = match d.status {
        ResultStatus::Pending => ("RUNNING", t.class(Class::Working)),
        ResultStatus::Ok => ("OK", t.ok()),
        ResultStatus::Error => ("ERROR", t.error()),
    };
    let time = |x: Option<Time>| {
        x.map(antfarm_view::fmt::clock)
            .unwrap_or_else(|| "-".into())
    };
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
    let info_h = lines.len() as u16 + 2;
    let [head, info, input, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(info_h),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    widgets::header(f, head, app, snap, now, t);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(Span::styled(" CALL ", t.label()))),
        info,
    );
    let body = call_body(&d);
    // Keep scrolling inside the text: count wrapped lines at the box's inner width.
    let inner_w = input.width.saturating_sub(2).max(1) as usize;
    let wrapped: usize = body
        .lines()
        .map(|l| l.chars().count().div_ceil(inner_w).max(1))
        .sum();
    let visible = input.height.saturating_sub(2) as usize;
    app.scroll = app.scroll.min(wrapped.saturating_sub(visible) as u16);
    f.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0))
            .block(
                Block::bordered()
                    .title(Span::styled(" INPUT · RESULT · secrets masked ", t.label())),
            ),
        input,
    );
    widgets::key_bar(f, keys, app, now, t);
}

/// The input, then the result, as one scrolling text.
fn call_body(d: &view::CallDetail) -> String {
    let mut body = d
        .input
        .clone()
        .unwrap_or_else(|| "The input could not be read from the transcript.".into());
    let rule = if d.status == ResultStatus::Error {
        "\n\n─── result · error ───\n"
    } else {
        "\n\n─── result ───\n"
    };
    body.push_str(rule);
    match (&d.result, d.status) {
        (Some(r), _) => {
            body.push_str(r);
            if d.result_cut {
                body.push_str(&format!(
                    "\n… cut at {} characters",
                    antfarm_core::claude::detail::RESULT_CHARS
                ));
            }
        }
        (None, ResultStatus::Pending) => body.push_str("No result yet."),
        (None, _) => body.push_str("The result could not be read from the transcript."),
    }
    body
}
