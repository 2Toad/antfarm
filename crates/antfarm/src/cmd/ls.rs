//! `antfarm ls`: one table of the sessions on this machine.

use antfarm_core::{AgentStatus, Session};
use comfy_table::presets::NOTHING;
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};

use super::{
    Scope, current, glyph, money, project_name, rank, settle, short_status, since, start_live,
    truncate,
};

#[derive(Debug, Default, clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub scope: Scope,
    /// Print the sessions as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    let live = start_live(false);
    settle(&live).await;
    let snap = live.handle.snapshot();
    live.handle.stop();
    let mut sessions: Vec<&Session> = snap
        .machine
        .sessions
        .values()
        .filter(|s| args.scope.keep(s))
        .collect();
    sessions.sort_by(|a, b| rank(a).cmp(&rank(b)).then(b.last_write.cmp(&a.last_write)));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("No Claude Code sessions running. Use --all to include exited ones.");
        return Ok(());
    }
    let now = chrono::Utc::now();
    let mut t = Table::new();
    t.load_style(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic);
    t.set_header([
        "",
        "PID",
        "NAME",
        "PROJECT",
        "BRANCH",
        "STATE",
        "FOR",
        "CURRENT / LAST CALL",
        "AGENTS",
        "CALLS",
        "COST",
        "CTX",
    ]);
    for s in sessions {
        let calls: u64 = s.agents.values().map(|a| a.calls).sum();
        let live_agents = s
            .agents
            .values()
            .filter(|a| {
                a.parent.is_some()
                    && matches!(
                        a.status,
                        AgentStatus::Working | AgentStatus::Thinking | AgentStatus::Waiting(_)
                    )
            })
            .count();
        let agents = if live_agents > 0 {
            format!("{} ({live_agents} live)", s.agents.len())
        } else {
            s.agents.len().to_string()
        };
        t.add_row(vec![
            Cell::new(glyph(s.status)),
            Cell::new(s.pid.map(|p| p.to_string()).unwrap_or_default())
                .set_alignment(CellAlignment::Right),
            Cell::new(truncate(s.name.as_deref().unwrap_or(s.id.as_str()), 24)),
            Cell::new(project_name(s)),
            Cell::new(truncate(s.branch.as_deref().unwrap_or("-"), 24)),
            Cell::new(short_status(s.status)),
            Cell::new(since(s.status_since, now)).set_alignment(CellAlignment::Right),
            Cell::new(truncate(&current(s), 60)),
            Cell::new(agents).set_alignment(CellAlignment::Right),
            Cell::new(calls).set_alignment(CellAlignment::Right),
            Cell::new(money(&s.cost)).set_alignment(CellAlignment::Right),
            Cell::new(
                s.context
                    .pct
                    .map(|p| format!("{p:.0}%"))
                    .unwrap_or_else(|| "-".into()),
            )
            .set_alignment(CellAlignment::Right),
        ]);
    }
    println!("{t}");
    Ok(())
}
