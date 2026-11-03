//! Formatting shared by the commands and the terminal UI.

use std::path::PathBuf;

use antfarm_core::{Cost, Session, SessionStatus, Time, WaitingKind};

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

/// `HH:MM:SS` in local time, or UTC under test so screen goldens do not depend on the
/// machine's time zone.
pub fn clock(t: Time) -> String {
    if cfg!(test) {
        t.format("%H:%M:%S").to_string()
    } else {
        t.with_timezone(&chrono::Local)
            .format("%H:%M:%S")
            .to_string()
    }
}

/// `HH:MM`, like [`clock`].
pub fn clock_hm(t: Time) -> String {
    if cfg!(test) {
        t.format("%H:%M").to_string()
    } else {
        t.with_timezone(&chrono::Local).format("%H:%M").to_string()
    }
}
