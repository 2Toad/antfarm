//! A usable PATH for an app launched from Finder or the Dock.
//!
//! macOS starts apps with only `/usr/bin:/bin:/usr/sbin:/sbin`, so go to terminal
//! can't find `tmux` or `code`. When none of the usual Homebrew directories are on
//! PATH, ask the login shell for its PATH once, and add those directories anyway.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const EXTRA: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];
const MARKER: &str = "__ANTFARM_PATH__";

/// Fixes PATH for this process. Call it before any other thread starts.
pub fn fix() {
    let current = std::env::var("PATH").unwrap_or_default();
    if current.split(':').any(|d| EXTRA.contains(&d)) {
        return;
    }
    let login = login_path(Duration::from_secs(3));
    // SAFETY: called at the top of `run`, before Tauri or Tokio start threads.
    unsafe { std::env::set_var("PATH", merged(&current, login.as_deref())) };
}

/// The login shell's PATH, or None if the shell fails or takes too long.
fn login_path(timeout: Duration) -> Option<String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let mut child = Command::new(shell)
        .args(["-lic", &format!("printf '{MARKER}%s' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout?.read_to_string(&mut out).ok()?;
    parse(&out)
}

/// The PATH after the marker; shell startup files may print before it.
fn parse(out: &str) -> Option<String> {
    let (_, path) = out.rsplit_once(MARKER)?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// The login PATH first, then what the process had, then the Homebrew
/// directories, without repeats.
fn merged(current: &str, login: Option<&str>) -> String {
    let mut out: Vec<&str> = Vec::new();
    for d in login
        .unwrap_or_default()
        .split(':')
        .chain(current.split(':'))
        .chain(EXTRA)
    {
        if !d.is_empty() && !out.contains(&d) {
            out.push(d);
        }
    }
    out.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_path_after_shell_noise() {
        let out = format!("Switching to Node LTS\n{MARKER}/opt/homebrew/bin:/usr/bin\n");
        assert_eq!(parse(&out).as_deref(), Some("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(parse("no marker here"), None);
        assert_eq!(parse(MARKER), None);
    }

    #[test]
    fn merges_login_path_first_without_repeats() {
        let finder = "/usr/bin:/bin:/usr/sbin:/sbin";
        assert_eq!(
            merged(
                finder,
                Some("/Users/me/.cargo/bin:/opt/homebrew/bin:/usr/bin")
            ),
            "/Users/me/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin"
        );
        assert_eq!(
            merged(finder, None),
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin"
        );
    }
}
