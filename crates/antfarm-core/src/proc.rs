//! Process liveness and ancestry, isolated behind a trait so tests and replay can fake it.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub trait ProcessProbe: Send {
    /// True when `pid` is running and, if `started_at_ms` is given, the process started
    /// no later than that. The start-time check rejects a reused pid.
    fn alive(&mut self, pid: u32, started_at_ms: Option<i64>) -> bool;

    /// `pid` and its parents, nearest first, as `(pid, executable path or name)`.
    fn ancestry(&mut self, _pid: u32) -> Vec<(u32, String)> {
        Vec::new()
    }
}

/// Allowed gap between the registry's start time and the OS process start time.
pub const START_TOLERANCE_SECS: i64 = 5;

/// Whether a process that started at `os_start_secs` can be the one that wrote a registry
/// entry with `started_at_ms`. Claude Code records `startedAt` only after its startup
/// prompts (folder trust, notices), so it can be any amount later than the process start.
/// A reused pid belongs to a process that started after the entry was written, so only a
/// start later than `startedAt` (plus clock slack) rules the entry out.
pub fn same_process(os_start_secs: i64, started_at_ms: i64) -> bool {
    os_start_secs <= started_at_ms / 1000 + START_TOLERANCE_SECS
}

/// Terminal multiplexers. When one is an ancestor it is where the session's pane lives,
/// so it wins over the app around it.
const MULTIPLEXERS: &[&str] = &["tmux", "zellij", "screen"];

/// Executable names that identify an app, for platforms without `.app` bundles.
const KNOWN: &[(&str, &str)] = &[
    ("kitty", "kitty"),
    ("alacritty", "Alacritty"),
    ("wezterm-gui", "WezTerm"),
    ("ghostty", "Ghostty"),
    ("gnome-terminal-server", "GNOME Terminal"),
    ("konsole", "Konsole"),
    ("code", "Visual Studio Code"),
    ("cursor", "Cursor"),
    ("zed", "Zed"),
];

fn base_name(exe: &str) -> String {
    exe.rsplit('/').next().unwrap_or(exe).to_ascii_lowercase()
}

/// The app a session runs in, from its ancestry (the session's own process first).
pub fn classify_app(ancestry: &[(u32, String)]) -> Option<String> {
    let parents = ancestry.iter().skip(1);
    for (_, exe) in parents.clone() {
        let base = base_name(exe);
        if let Some(m) = MULTIPLEXERS
            .iter()
            .find(|m| base == **m || base.starts_with(&format!("{m}:")))
        {
            return Some((*m).to_owned());
        }
    }
    for (_, exe) in parents {
        if let Some(app) = bundle_name(exe) {
            return Some(app);
        }
        let base = base_name(exe);
        if let Some((_, app)) = KNOWN.iter().find(|(name, _)| base == *name) {
            return Some((*app).to_owned());
        }
    }
    None
}

/// The outermost `.app` bundle in a macOS executable path, which is also the name
/// `open -a` takes: `/Applications/Visual Studio Code.app/Contents/Frameworks/Code
/// Helper (Plugin).app/...` gives `Visual Studio Code`.
fn bundle_name(exe: &str) -> Option<String> {
    let end = exe.find(".app/")?;
    let start = exe[..end].rfind('/').map_or(0, |i| i + 1);
    Some(exe[start..end].to_owned())
}

pub struct SysinfoProbe {
    /// `None` when the platform probe could not start; every pid then counts as alive,
    /// since calling live sessions exited would be worse than missing a dead one.
    sys: Option<System>,
}

impl Default for SysinfoProbe {
    fn default() -> Self {
        Self {
            sys: System::new().ok(),
        }
    }
}

/// OS start time of `pid` in seconds since the epoch.
fn start_time(sys: &mut System, pid: u32) -> Option<u64> {
    let p = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[p]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(p).map(|proc| proc.start_time())
}

impl ProcessProbe for SysinfoProbe {
    fn alive(&mut self, pid: u32, started_at_ms: Option<i64>) -> bool {
        let Some(sys) = self.sys.as_mut() else {
            return true;
        };
        let Some(start) = start_time(sys, pid) else {
            return false;
        };
        match started_at_ms {
            Some(ms) => same_process(start as i64, ms),
            None => true,
        }
    }

    fn ancestry(&mut self, pid: u32) -> Vec<(u32, String)> {
        let Some(sys) = self.sys.as_mut() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut cur = Some(Pid::from_u32(pid));
        while let Some(p) = cur.filter(|p| p.as_u32() > 1 && out.len() < 32) {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[p]),
                true,
                ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
            );
            let Some(proc) = sys.process(p) else { break };
            let exe = proc
                .exe()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_else(|| proc.name().to_string_lossy().into_owned());
            out.push((p.as_u32(), exe));
            cur = proc.parent();
        }
        out
    }
}

/// A probe that answers from fixed lists, for tests.
#[derive(Debug, Default, Clone)]
pub struct FakeProbe {
    pub alive: Vec<u32>,
    /// Ancestry per pid, nearest first, starting with the pid itself.
    pub ancestry: Vec<(u32, Vec<(u32, String)>)>,
}

impl ProcessProbe for FakeProbe {
    fn alive(&mut self, pid: u32, _: Option<i64>) -> bool {
        self.alive.contains(&pid)
    }

    fn ancestry(&mut self, pid: u32) -> Vec<(u32, String)> {
        self.ancestry
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, a)| a.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_start_time_matches() {
        let mut sys = System::new().unwrap();
        let me = std::process::id();
        let start = start_time(&mut sys, me).unwrap() as i64 * 1000;
        let mut probe = SysinfoProbe::default();
        assert!(probe.alive(me, None));
        assert!(probe.alive(me, Some(start + 1500)));
        assert!(
            !probe.alive(me, Some(start - 3_600_000)),
            "a reused pid must not count as alive"
        );
    }

    #[test]
    fn dead_pid_is_not_alive() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!SysinfoProbe::default().alive(pid, None));
    }

    #[test]
    fn this_process_has_an_ancestry() {
        let a = SysinfoProbe::default().ancestry(std::process::id());
        assert_eq!(a.first().map(|x| x.0), Some(std::process::id()));
        assert!(a.len() >= 2, "{a:?}");
    }

    fn chain(exes: &[&str]) -> Vec<(u32, String)> {
        exes.iter()
            .enumerate()
            .map(|(i, e)| (100 + i as u32, (*e).to_owned()))
            .collect()
    }

    #[test]
    fn classifies_apps_from_ancestry() {
        let claude =
            "/Users/x/.vscode/extensions/anthropic.claude-code/resources/native-binary/claude";
        let helper = "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper (Plugin).app/Contents/MacOS/Code Helper (Plugin)";
        let code = "/Applications/Visual Studio Code.app/Contents/MacOS/Code";
        assert_eq!(
            classify_app(&chain(&[claude, helper, code])).as_deref(),
            Some("Visual Studio Code")
        );
        let iterm = "/Applications/iTerm.app/Contents/MacOS/iTerm2";
        assert_eq!(
            classify_app(&chain(&[
                "claude",
                "/bin/zsh",
                "/opt/homebrew/bin/tmux",
                iterm
            ]))
            .as_deref(),
            Some("tmux")
        );
        assert_eq!(
            classify_app(&chain(&["claude", "/bin/zsh", "/usr/bin/login", iterm])).as_deref(),
            Some("iTerm")
        );
        assert_eq!(
            classify_app(&chain(&["claude", "bash", "/usr/bin/kitty"])).as_deref(),
            Some("kitty")
        );
        assert_eq!(classify_app(&chain(&["claude", "bash", "sshd"])), None);
        assert_eq!(
            classify_app(&chain(&["/usr/bin/tmux"])),
            None,
            "the session itself is not its own app"
        );
    }

    #[test]
    fn a_late_registry_entry_still_matches_its_process() {
        let started_at_ms = 1_000_000_000;
        // Started 11 s before the entry, as when the folder-trust prompt sat open.
        assert!(same_process(1_000_000 - 11, started_at_ms));
        assert!(same_process(1_000_000 - 3_600, started_at_ms));
        // Clock slack either side of the entry.
        assert!(same_process(
            1_000_000 + START_TOLERANCE_SECS,
            started_at_ms
        ));
        // A pid reused by a process started after the entry was written.
        assert!(!same_process(
            1_000_000 + START_TOLERANCE_SECS + 1,
            started_at_ms
        ));
        assert!(!same_process(1_000_000 + 86_400, started_at_ms));
    }
}
