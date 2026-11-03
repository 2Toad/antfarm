//! Pieces shared by the subcommands: starting the live source, waiting for it to settle,
//! filtering sessions, and formatting.

pub mod doctor;
pub mod ls;
pub mod replay;
pub mod watch;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use antfarm_core::claude::{ClaudeCodeSource, SourceConfig, SourceStats};
use antfarm_core::source::{Clock, Handle, Hub};
use antfarm_core::{Cost, Paths, Session, SessionStatus, Time, WaitingKind};

pub struct Live {
    pub handle: Handle,
    pub stats: Arc<Mutex<SourceStats>>,
}

pub fn start_live() -> Live {
    let source = ClaudeCodeSource::new(SourceConfig::new(Paths::from_env()));
    let stats = source.stats();
    let handle = Hub::spawn(Box::new(source), Clock::Wall);
    Live { handle, stats }
}

/// Waits until the source has made two full passes and the snapshot has been quiet for a
/// moment, or five seconds, whichever comes first.
pub async fn settle(live: &Live) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while live.stats.lock().unwrap().passes < 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut rx = live.handle.snapshots();
    rx.borrow_and_update();
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(300), rx.changed()).await {
            Ok(Ok(())) => continue,
            _ => break,
        }
    }
}

/// Which sessions a command shows.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct Scope {
    /// Only sessions in the current directory's project.
    #[arg(long, short = 'H')]
    pub here: bool,
    /// Only the session whose id starts with this.
    #[arg(long)]
    pub session: Option<String>,
    /// Include sessions whose process is gone.
    #[arg(long, short)]
    pub all: bool,
}

impl Scope {
    pub fn keep(&self, s: &Session) -> bool {
        if !self.all && s.status == SessionStatus::Exited && self.session.is_none() {
            return false;
        }
        if let Some(prefix) = &self.session
            && !s.id.as_str().starts_with(prefix.as_str())
        {
            return false;
        }
        if self.here {
            let Ok(cwd) = std::env::current_dir() else {
                return true;
            };
            return same_project(&cwd, &s.project);
        }
        true
    }

    pub fn keep_id(&self, id: Option<&str>) -> bool {
        match (&self.session, id) {
            (Some(p), Some(id)) => id.starts_with(p.as_str()),
            (Some(_), None) => false,
            _ => true,
        }
    }
}

fn same_project(cwd: &std::path::Path, project: &std::path::Path) -> bool {
    !project.as_os_str().is_empty() && (cwd.starts_with(project) || project.starts_with(cwd))
}

/// Sort key: sessions that need a human first, then active, then quiet, then gone.
pub fn rank(s: &Session) -> u8 {
    match s.status {
        SessionStatus::Waiting(_) => 0,
        SessionStatus::Working | SessionStatus::Thinking => 1,
        SessionStatus::Still => 2,
        SessionStatus::Idle => 3,
        SessionStatus::Exited => 4,
    }
}

pub fn glyph(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Waiting(_) => "◆",
        SessionStatus::Working | SessionStatus::Thinking => "●",
        SessionStatus::Idle => "✓",
        SessionStatus::Still => "○",
        SessionStatus::Exited => "–",
    }
}

pub fn short_status(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Waiting(WaitingKind::Permission) => "PERMIT",
        SessionStatus::Waiting(WaitingKind::Question) => "ASK",
        SessionStatus::Working => "working",
        SessionStatus::Thinking => "thinking",
        SessionStatus::Idle => "idle",
        SessionStatus::Still => "still",
        SessionStatus::Exited => "exited",
    }
}

/// `45s`, `4m12s`, `1h03m`, `2d4h`.
pub fn dur(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m{:02}s", s / 60, s % 60),
        3600..86400 => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
        _ => format!("{}d{}h", s / 86400, (s % 86400) / 3600),
    }
}

pub fn since(t: Option<Time>, now: Time) -> String {
    t.map(|t| dur((now - t).num_seconds()))
        .unwrap_or_else(|| "-".into())
}

pub fn money(c: &Cost) -> String {
    let total = c.total();
    if total == 0.0 && c.reported_usd.is_none() {
        return "-".into();
    }
    let est = if c.is_estimate() { "~" } else { "" };
    format!("{est}${total:.2}")
}

/// The call the session is blocked on or busy with, else its most recent call.
pub fn current(s: &Session) -> String {
    let mut open: Vec<_> = s.open_calls().collect();
    open.sort_by_key(|c| c.started);
    if let Some(c) = open.last() {
        return format!("{} {}", c.name, c.summary);
    }
    s.agents
        .values()
        .flat_map(|a| a.activity.iter())
        .filter_map(|a| match a {
            antfarm_core::Activity::Call {
                name, summary, at, ..
            } => Some((at, name, summary)),
            _ => None,
        })
        .max_by_key(|(at, ..)| **at)
        .map(|(_, n, s)| format!("{n} {s}"))
        .unwrap_or_default()
}

pub fn project_name(s: &Session) -> String {
    s.project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "-".into())
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn home_relative(p: &std::path::Path) -> String {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    match p.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}
