//! Pieces shared by the subcommands: starting the live source, waiting for it to settle,
//! filtering sessions, and formatting.

pub mod doctor;
pub mod ls;
pub mod replay;
pub mod watch;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use antfarm_core::claude::{ClaudeCodeSource, SourceConfig, SourceStats, Wants};
use antfarm_core::source::{Clock, Handle, Hub};
use antfarm_core::{Paths, Session, SessionStatus};

pub use antfarm_view::fmt::*;

pub struct Live {
    pub handle: Handle,
    pub stats: Arc<Mutex<SourceStats>>,
    /// Asks the source for old sub-agents' transcripts when their screen opens.
    pub wants: Wants,
}

pub fn start_live() -> Live {
    let source = ClaudeCodeSource::new(SourceConfig::new(Paths::from_env()));
    let stats = source.stats();
    let wants = source.wants();
    let handle = Hub::spawn(Box::new(source), Clock::Wall);
    Live {
        handle,
        stats,
        wants,
    }
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
