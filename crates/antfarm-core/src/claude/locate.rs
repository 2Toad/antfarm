//! Finds a session's transcript and its sub-agent and workflow directories.
//!
//! The project slug Claude Code derives from `cwd` is undocumented, so it is only a
//! first guess. The authority is a search of every project directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug)]
pub struct Locator {
    projects: PathBuf,
    cache: HashMap<String, PathBuf>,
}

/// Directories that belong to one session, possibly spread across several slugs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionDirs {
    pub subagents: Vec<PathBuf>,
    pub workflows: Vec<PathBuf>,
}

#[derive(Deserialize)]
struct SessionsIndex {
    #[serde(default)]
    entries: Vec<IndexEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexEntry {
    session_id: String,
    full_path: Option<PathBuf>,
}

/// Claude Code's slug: every character other than an ASCII letter or digit becomes `-`.
pub fn slug_guess(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

impl Locator {
    pub fn new(projects: impl Into<PathBuf>) -> Self {
        Self {
            projects: projects.into(),
            cache: HashMap::new(),
        }
    }

    /// The main transcript for `session_id`, or `None` if it does not exist yet.
    pub fn transcript(&mut self, session_id: &str, cwd: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = self.cache.get(session_id) {
            if p.is_file() {
                return Some(p.clone());
            }
            self.cache.remove(session_id);
        }
        let file = format!("{session_id}.jsonl");
        let found = cwd
            .map(|c| self.projects.join(slug_guess(c)))
            .and_then(|dir| {
                let direct = dir.join(&file);
                if direct.is_file() {
                    return Some(direct);
                }
                index_lookup(&dir, session_id)
            })
            .or_else(|| self.glob_one(&file));
        if let Some(p) = &found {
            self.cache.insert(session_id.to_owned(), p.clone());
        }
        found
    }

    fn glob_one(&self, file: &str) -> Option<PathBuf> {
        let pattern = self.projects.join("*").join(glob::Pattern::escape(file));
        glob::glob(pattern.to_str()?)
            .ok()?
            .flatten()
            .find(|p| p.is_file())
    }

    /// Every `<slug>/<sessionId>/subagents` and `<slug>/<sessionId>/workflows` directory.
    pub fn session_dirs(&self, session_id: &str) -> SessionDirs {
        let mut out = SessionDirs::default();
        let pattern = self
            .projects
            .join("*")
            .join(glob::Pattern::escape(session_id));
        let Some(pattern) = pattern.to_str() else {
            return out;
        };
        let Ok(paths) = glob::glob(pattern) else {
            return out;
        };
        for dir in paths.flatten().filter(|p| p.is_dir()) {
            let s = dir.join("subagents");
            if s.is_dir() {
                out.subagents.push(s);
            }
            let w = dir.join("workflows");
            if w.is_dir() {
                out.workflows.push(w);
            }
        }
        out.subagents.sort();
        out.workflows.sort();
        out
    }
}

fn index_lookup(dir: &Path, session_id: &str) -> Option<PathBuf> {
    let bytes = std::fs::read(dir.join("sessions-index.json")).ok()?;
    let idx: SessionsIndex = serde_json::from_slice(&bytes).ok()?;
    idx.entries
        .into_iter()
        .find(|e| e.session_id == session_id)
        .and_then(|e| e.full_path)
        .filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_matches_claude_code() {
        assert_eq!(
            slug_guess("/Users/jason/2toad/projects/antfarm"),
            "-Users-jason-2toad-projects-antfarm"
        );
        assert_eq!(
            slug_guess("/a/my-app/.claude/worktrees/x_y"),
            "-a-my-app--claude-worktrees-x-y"
        );
    }

    #[test]
    fn finds_transcript_by_guess_index_and_search_and_merges_session_dirs() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        let main = p.join("-w-app");
        let wt = p.join("-w-app--claude-worktrees-feature");
        let other = p.join("-elsewhere");
        for d in [&main, &wt, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(main.join("s1.jsonl"), "").unwrap();
        std::fs::write(other.join("s2.jsonl"), "").unwrap();
        std::fs::write(other.join("s3.jsonl"), "").unwrap();
        std::fs::write(
            main.join("sessions-index.json"),
            format!(
                r#"{{"version":1,"entries":[{{"sessionId":"s3","fullPath":"{}"}}]}}"#,
                other.join("s3.jsonl").display()
            ),
        )
        .unwrap();
        std::fs::create_dir_all(main.join("s1/subagents")).unwrap();
        std::fs::create_dir_all(wt.join("s1/workflows")).unwrap();
        std::fs::create_dir_all(wt.join("s1/subagents")).unwrap();

        let mut loc = Locator::new(p);
        assert_eq!(
            loc.transcript("s1", Some("/w/app")),
            Some(main.join("s1.jsonl"))
        );
        assert_eq!(
            loc.transcript("s2", Some("/w/app")),
            Some(other.join("s2.jsonl"))
        );
        assert_eq!(
            loc.transcript("s3", Some("/w/app")),
            Some(other.join("s3.jsonl"))
        );
        assert_eq!(loc.transcript("s9", None), None);

        let dirs = loc.session_dirs("s1");
        assert_eq!(
            dirs.subagents,
            [main.join("s1/subagents"), wt.join("s1/subagents")]
        );
        assert_eq!(dirs.workflows, [wt.join("s1/workflows")]);
    }
}
