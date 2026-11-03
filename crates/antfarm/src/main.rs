//! antfarm: see what every Claude Code agent and sub-agent on this machine is doing.

mod cmd;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "antfarm",
    version,
    about = "See what every Claude Code agent and sub-agent on this machine is doing, live"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List sessions once and exit.
    Ls(cmd::ls::Args),
    /// Stream events, or snapshots and events as NDJSON with --json.
    Watch(cmd::watch::Args),
    /// Report what antfarm found on disk and anything it could not read.
    Doctor(cmd::doctor::Args),
    /// Play a recorded transcript back through the same engine.
    Replay(cmd::replay::Args),
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("ANTFARM_LOG"))
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match cli.command {
            // The terminal UI is the next slice; until then the bare command lists sessions.
            None => cmd::ls::run(cmd::ls::Args::default()).await,
            Some(Command::Ls(a)) => cmd::ls::run(a).await,
            Some(Command::Watch(a)) => cmd::watch::run(a).await,
            Some(Command::Doctor(a)) => cmd::doctor::run(a).await,
            Some(Command::Replay(a)) => cmd::replay::run(a).await,
        }
    })
}
