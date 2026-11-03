//! antfarm-core builds a live model of the Claude Code sessions on a machine from the
//! files Claude Code already writes under `~/.claude`.

use std::path::PathBuf;

/// Where Claude Code keeps its state. Defaults to `$HOME/.claude`, overridable with
/// `ANTFARM_CLAUDE_HOME` for tests and unusual setups.
#[derive(Debug, Clone)]
pub struct Paths {
    pub claude_home: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let claude_home = std::env::var_os("ANTFARM_CLAUDE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
                home.join(".claude")
            });
        Self { claude_home }
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.claude_home.join("sessions")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.claude_home.join("projects")
    }
}
