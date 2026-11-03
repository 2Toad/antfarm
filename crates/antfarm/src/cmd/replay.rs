//! `antfarm replay`: play a recorded transcript back through the same engine.

use std::io::Write;
use std::path::PathBuf;

use antfarm_core::EventEnvelope;
use antfarm_core::replay::{ReplayConfig, ReplaySource, Speed, parse_speed};
use antfarm_core::source::{Clock, Hub};
use tokio::sync::broadcast::error::TryRecvError;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A main transcript (`<sessionId>.jsonl`); sub-agents and workflows beside it are found.
    pub file: PathBuf,
    /// `max`, or how many times faster than recorded, such as `10`.
    #[arg(long, default_value = "10", value_parser = parse_speed)]
    pub speed: Speed,
    /// Longest real pause between two lines, in seconds.
    #[arg(long, default_value_t = 3.0)]
    pub max_gap: f64,
    /// Emit NDJSON events and a final snapshot.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    if !args.file.is_file() {
        anyhow::bail!("{} is not a file", args.file.display());
    }
    let mut config = ReplayConfig::new(&args.file);
    config.speed = args.speed;
    config.max_gap = std::time::Duration::from_secs_f64(args.max_gap.max(0.0));
    let handle = Hub::spawn(Box::new(ReplaySource::new(config)), Clock::Source);
    let mut rx = handle.events();
    let print = |e: &EventEnvelope| -> std::io::Result<()> {
        let mut o = std::io::stdout().lock();
        if args.json {
            writeln!(o, "{}", serde_json::json!({"kind": "event", "event": e}))
        } else if let Some(line) = super::watch::describe(e) {
            writeln!(
                o,
                "{}  {}",
                e.at.with_timezone(&chrono::Local).format("%H:%M:%S"),
                line
            )
        } else {
            Ok(())
        }
    };
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { handle.stop(); break }
            _ = handle.finished() => break,
            r = rx.recv() => match r {
                Ok(e) => print(&e)?,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("skipped {n} events"),
                Err(_) => break,
            },
        }
    }
    loop {
        match rx.try_recv() {
            Ok(e) => print(&e)?,
            Err(TryRecvError::Lagged(n)) => eprintln!("skipped {n} events"),
            Err(_) => break,
        }
    }
    let snap = handle.snapshot();
    if args.json {
        let sessions: Vec<_> = snap.machine.sessions.values().collect();
        writeln!(
            std::io::stdout().lock(),
            "{}",
            serde_json::json!({"kind": "snapshot", "seq": snap.seq, "at": snap.at, "sessions": sessions})
        )?;
    } else if let Some(s) = snap.machine.sessions.values().next() {
        println!();
        println!(
            "{}: {} agents, {} calls, {} at the end, {}",
            s.name.as_deref().unwrap_or(s.id.as_str()),
            s.agents.len(),
            s.agents.values().map(|a| a.calls).sum::<u64>(),
            s.status.label(),
            super::money(&s.cost)
        );
    }
    handle.join().await?;
    Ok(())
}
