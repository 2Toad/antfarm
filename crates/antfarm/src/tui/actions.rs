//! Side effects the TUI asks for: bringing a session's terminal forward and copying text.
//! Choosing what to run is a pure function so it can be tested; running it is a thin
//! wrapper around `std::process::Command`.

use std::io::Write;
use std::path::PathBuf;

use antfarm_core::Session;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `session:window.pane`.
    Tmux(String),
    /// An app to bring forward, and a folder to open in it for editors.
    App {
        app: String,
        folder: Option<PathBuf>,
    },
    /// Nothing to switch to, and why.
    None(String),
}

const EDITORS: &[&str] = &["Visual Studio Code", "Cursor", "Zed"];

pub fn target(s: &Session) -> Target {
    let app = match (&s.source, s.entrypoint.as_deref()) {
        (Some(src), _) => {
            if let Some(p) = &src.tmux_pane {
                return Target::Tmux(p.clone());
            }
            src.app.clone()
        }
        (None, Some("claude-vscode")) => "Visual Studio Code".to_owned(),
        (None, _) => {
            return Target::None("antfarm could not tell which app this session runs in".into());
        }
    };
    if app == "tmux" {
        return Target::None("this session is in tmux, but its pane was not found".into());
    }
    let folder = EDITORS
        .contains(&app.as_str())
        .then(|| s.project.clone())
        .filter(|p| !p.as_os_str().is_empty());
    Target::App { app, folder }
}

/// The commands that bring `t` forward, in order.
pub fn commands(t: &Target, macos: bool) -> Result<Vec<Vec<String>>, String> {
    match t {
        Target::Tmux(pane) => {
            let session = pane.split(':').next().unwrap_or(pane).to_owned();
            let window = pane
                .rsplit_once('.')
                .map(|(w, _)| w.to_owned())
                .unwrap_or_else(|| pane.clone());
            Ok(vec![
                vec!["tmux".into(), "switch-client".into(), "-t".into(), session],
                vec!["tmux".into(), "select-window".into(), "-t".into(), window],
                vec![
                    "tmux".into(),
                    "select-pane".into(),
                    "-t".into(),
                    pane.clone(),
                ],
            ])
        }
        Target::App { app, folder } if macos => {
            let mut c = vec!["open".into(), "-a".into(), app.clone()];
            if let Some(f) = folder {
                c.push(f.display().to_string());
            }
            Ok(vec![c])
        }
        Target::App {
            app,
            folder: Some(f),
        } if app == "Visual Studio Code" => Ok(vec![vec![
            "code".into(),
            "-r".into(),
            f.display().to_string(),
        ]]),
        Target::App { app, .. } => Err(format!("switching to {app} is only supported on macOS")),
        Target::None(why) => Err(why.clone()),
    }
}

pub fn go(s: &Session) -> Result<String, String> {
    let t = target(s);
    for c in commands(&t, cfg!(target_os = "macos"))? {
        let out = std::process::Command::new(&c[0])
            .args(&c[1..])
            .output()
            .map_err(|e| format!("{}: {e}", c[0]))?;
        // `switch-client` fails outside tmux; selecting the window and pane still helps.
        if !out.status.success() && c.get(1).map(String::as_str) != Some("switch-client") {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
    }
    Ok(match t {
        Target::Tmux(p) => format!("switched to tmux pane {p}"),
        Target::App { app, .. } => format!("brought {app} forward"),
        Target::None(_) => unreachable!("commands() refuses None"),
    })
}

/// Copies through OSC 52, which works over SSH and in most terminals, and through the
/// platform clipboard tool when there is one.
pub fn copy(text: &str) -> Result<String, String> {
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
    let tools: &[&[&str]] = &[
        &["pbcopy"],
        &["wl-copy"],
        &["xclip", "-selection", "clipboard"],
    ];
    for tool in tools {
        let Ok(mut child) = std::process::Command::new(tool[0])
            .args(&tool[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().is_ok_and(|s| s.success()) {
            return Ok(format!("copied {} characters", text.chars().count()));
        }
    }
    Ok(format!(
        "copied {} characters (terminal clipboard)",
        text.chars().count()
    ))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use antfarm_core::{SessionId, SourceApp};

    fn session(app: Option<&str>, pane: Option<&str>, entry: Option<&str>) -> Session {
        let mut s = Session::new(SessionId::from("s"), "/w/app".into());
        s.source = app.map(|a| SourceApp {
            app: a.into(),
            tmux_pane: pane.map(Into::into),
        });
        s.entrypoint = entry.map(Into::into);
        s
    }

    #[test]
    fn resolves_targets() {
        assert_eq!(
            target(&session(Some("tmux"), Some("work:2.1"), None)),
            Target::Tmux("work:2.1".into())
        );
        assert!(matches!(
            target(&session(Some("tmux"), None, None)),
            Target::None(_)
        ));
        assert_eq!(
            target(&session(None, None, Some("claude-vscode"))),
            Target::App {
                app: "Visual Studio Code".into(),
                folder: Some("/w/app".into())
            }
        );
        assert_eq!(
            target(&session(Some("iTerm"), None, Some("cli"))),
            Target::App {
                app: "iTerm".into(),
                folder: None
            }
        );
        assert!(matches!(
            target(&session(None, None, Some("cli"))),
            Target::None(_)
        ));
    }

    #[test]
    fn builds_commands() {
        let tmux = commands(&Target::Tmux("work:2.1".into()), true).unwrap();
        assert_eq!(tmux[0], ["tmux", "switch-client", "-t", "work"]);
        assert_eq!(tmux[1], ["tmux", "select-window", "-t", "work:2"]);
        assert_eq!(tmux[2], ["tmux", "select-pane", "-t", "work:2.1"]);
        let vsc = Target::App {
            app: "Visual Studio Code".into(),
            folder: Some("/w/app".into()),
        };
        assert_eq!(
            commands(&vsc, true).unwrap(),
            [["open", "-a", "Visual Studio Code", "/w/app"]]
        );
        assert_eq!(commands(&vsc, false).unwrap(), [["code", "-r", "/w/app"]]);
        assert!(
            commands(
                &Target::App {
                    app: "iTerm".into(),
                    folder: None
                },
                false
            )
            .is_err()
        );
    }

    #[test]
    fn base64_matches_rfc_examples() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
