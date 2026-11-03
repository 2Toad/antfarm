//! Process liveness, isolated behind a trait so tests and replay can fake it.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub trait ProcessProbe: Send {
    /// True when `pid` is running and, if `started_at_ms` is given, started within five
    /// seconds of it. The start-time check rejects a reused pid.
    fn alive(&mut self, pid: u32, started_at_ms: Option<i64>) -> bool;
}

/// Allowed gap between the registry's start time and the OS process start time.
pub const START_TOLERANCE_SECS: i64 = 5;

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
            Some(ms) => (start as i64 - ms / 1000).abs() <= START_TOLERANCE_SECS,
            None => true,
        }
    }
}

/// A probe that answers from a fixed list, for tests.
#[derive(Debug, Default, Clone)]
pub struct FakeProbe {
    pub alive: Vec<u32>,
}

impl ProcessProbe for FakeProbe {
    fn alive(&mut self, pid: u32, _: Option<i64>) -> bool {
        self.alive.contains(&pid)
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
}
