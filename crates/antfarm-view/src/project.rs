//! The project view: a repository's sessions side by side, the files they touched
//! recently, and where two sessions edited the same file in the same working tree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use antfarm_core::{Session, SessionId, Snapshot, Time};
use serde::Serialize;

use crate::fmt::{dur, money, project_name};
use crate::view::{self, AgentLine, Class};

/// How far back "recently touched" reaches, and the window inside which two edits of
/// the same file in the same tree count as a conflict.
pub const RECENT_SECS: i64 = 600;

/// The repository a session belongs to, falling back to its project directory.
pub fn repo_key(s: &Session) -> PathBuf {
    s.repo.clone().unwrap_or_else(|| s.project.clone())
}

/// The working tree a session runs in, falling back to its project directory.
pub fn tree_key(s: &Session) -> PathBuf {
    s.worktree.clone().unwrap_or_else(|| s.project.clone())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectSummary {
    pub key: String,
    pub name: String,
    pub sessions: usize,
    pub agents: usize,
    pub waiting: usize,
}

/// Every repository with a live session, in order of first appearance.
pub fn projects(snap: &Snapshot) -> Vec<ProjectSummary> {
    let mut out: Vec<ProjectSummary> = Vec::new();
    for s in snap
        .machine
        .sessions
        .values()
        .filter(|s| s.status != antfarm_core::SessionStatus::Exited)
    {
        let key = repo_key(s).display().to_string();
        let i = match out.iter().position(|p| p.key == key) {
            Some(i) => i,
            None => {
                out.push(ProjectSummary {
                    key: key.clone(),
                    name: name_of(&repo_key(s), s),
                    sessions: 0,
                    agents: 0,
                    waiting: 0,
                });
                out.len() - 1
            }
        };
        out[i].sessions += 1;
        out[i].agents += s.agents.len();
        out[i].waiting += usize::from(s.status.needs_human());
    }
    out
}

fn name_of(p: &Path, s: &Session) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| project_name(s))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectSession {
    pub id: SessionId,
    pub name: String,
    pub class: Class,
    pub state: String,
    pub for_secs: Option<i64>,
    pub branch: Option<String>,
    pub tree: String,
    pub worktree: bool,
    pub source: String,
    pub pid: Option<u32>,
    pub carrying: usize,
    pub added: u64,
    pub removed: u64,
    pub cost: String,
    pub usd_per_min: f64,
    /// Files this session shares with another session in the same tree.
    pub shares: Vec<String>,
    pub agents: Vec<AgentLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Touch {
    pub session: SessionId,
    pub session_name: String,
    pub agent: String,
    pub tool: String,
    pub at: Option<Time>,
    pub added: u64,
    pub removed: u64,
    pub worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileRow {
    pub path: String,
    pub last_at: Option<Time>,
    pub touches: Vec<Touch>,
    /// Set when two sessions in the same working tree touched it inside the window.
    pub conflict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BranchRow {
    pub branch: String,
    pub tree: String,
    pub sessions: usize,
    pub conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectView {
    pub key: String,
    pub name: String,
    pub sessions: Vec<ProjectSession>,
    pub files: Vec<FileRow>,
    pub branches: Vec<BranchRow>,
    pub agents: usize,
    pub extra_worktrees: usize,
    pub calls_per_min: f64,
    pub cost: f64,
    pub conflicts: usize,
}

pub fn project_view(snap: &Snapshot, key: &str, now: Time) -> Option<ProjectView> {
    let sessions: Vec<&Session> = snap
        .machine
        .sessions
        .values()
        .filter(|s| {
            s.status != antfarm_core::SessionStatus::Exited
                && repo_key(s).display().to_string() == key
        })
        .collect();
    let first = sessions.first()?;
    let repo = repo_key(first);
    let since = now - chrono::Duration::seconds(RECENT_SECS);

    // Recent touches per path, newest first within each path.
    let mut by_path: BTreeMap<String, Vec<(&Session, &antfarm_core::FileTouch)>> = BTreeMap::new();
    for s in &sessions {
        for f in s
            .files_touched
            .iter()
            .filter(|f| f.at.is_some_and(|t| t >= since))
        {
            by_path.entry(f.path.clone()).or_default().push((s, f));
        }
    }
    let mut files: Vec<FileRow> = by_path
        .into_iter()
        .map(|(path, mut touches)| {
            touches.sort_by_key(|(_, f)| std::cmp::Reverse(f.at));
            let conflict = conflict_text(&path, &touches);
            FileRow {
                path,
                last_at: touches.first().and_then(|(_, f)| f.at),
                touches: touches
                    .iter()
                    .map(|(s, f)| Touch {
                        session: s.id.clone(),
                        session_name: view::session_name(s),
                        agent: s
                            .agents
                            .get(&f.agent)
                            .map(view::agent_name)
                            .unwrap_or_else(|| "root".into()),
                        tool: f.tool.clone(),
                        at: f.at,
                        added: f.added,
                        removed: f.removed,
                        worktree: tree_key(s) != repo,
                    })
                    .collect(),
                conflict,
            }
        })
        .collect();
    files.sort_by(|a, b| {
        b.conflict
            .is_some()
            .cmp(&a.conflict.is_some())
            .then(b.last_at.cmp(&a.last_at))
    });

    let shared: Vec<(&str, &SessionId)> = files
        .iter()
        .filter(|f| f.conflict.is_some())
        .flat_map(|f| f.touches.iter().map(move |t| (f.path.as_str(), &t.session)))
        .collect();
    let mut cards: Vec<ProjectSession> = sessions
        .iter()
        .map(|s| {
            let open = s.open_calls().max_by_key(|c| c.started).map(|c| c.category);
            ProjectSession {
                id: s.id.clone(),
                name: view::session_name(s),
                class: Class::of_session(s.status),
                state: view::state_word(s.status, open).to_owned(),
                for_secs: s.status_since.map(|t| (now - t).num_seconds().max(0)),
                branch: s.branch.clone(),
                tree: tree_key(s).display().to_string(),
                worktree: tree_key(s) != repo,
                source: view::source_label(s),
                pid: s.pid,
                carrying: s.files_touched.len(),
                added: s.files_touched.iter().map(|f| f.added).sum(),
                removed: s.files_touched.iter().map(|f| f.removed).sum(),
                cost: money(&s.cost),
                usd_per_min: s.cost.usd_per_min,
                shares: shared
                    .iter()
                    .filter(|(_, id)| **id == s.id)
                    .map(|(p, _)| file_name(p))
                    .collect(),
                // Live and finished sub-agents; stale ones would only crowd the card.
                agents: view::agent_lines(s, now)
                    .into_iter()
                    .filter(|a| {
                        a.depth > 0
                            && matches!(a.class, Class::Working | Class::Waiting | Class::Finished)
                    })
                    .take(8)
                    .collect(),
            }
        })
        .collect();
    cards.sort_by(|a, b| {
        crate::fmt::rank_class(a.class)
            .cmp(&crate::fmt::rank_class(b.class))
            .then(b.shares.len().cmp(&a.shares.len()))
    });

    let mut branches: Vec<BranchRow> = Vec::new();
    for s in &sessions {
        let branch = s.branch.clone().unwrap_or_else(|| "-".into());
        let tree = tree_key(s).display().to_string();
        match branches
            .iter_mut()
            .find(|b| b.branch == branch && b.tree == tree)
        {
            Some(b) => b.sessions += 1,
            None => branches.push(BranchRow {
                branch,
                tree,
                sessions: 1,
                conflict: false,
            }),
        }
    }
    for b in &mut branches {
        b.conflict = files.iter().any(|f| {
            f.conflict.is_some()
                && f.touches.iter().any(|t| {
                    sessions
                        .iter()
                        .any(|s| s.id == t.session && tree_key(s).display().to_string() == b.tree)
                })
        });
    }
    let trees: std::collections::BTreeSet<String> = sessions
        .iter()
        .map(|s| tree_key(s).display().to_string())
        .collect();
    let agents: Vec<&antfarm_core::Agent> =
        sessions.iter().flat_map(|s| s.agents.values()).collect();
    let calls: u32 = agents
        .iter()
        .flat_map(|a| a.minutes.iter())
        .filter(|b| (now - b.start).num_minutes() < 5)
        .map(|b| b.calls)
        .sum();
    Some(ProjectView {
        key: key.to_owned(),
        name: name_of(&repo, first),
        conflicts: files.iter().filter(|f| f.conflict.is_some()).count(),
        sessions: cards,
        files,
        branches,
        agents: agents.len(),
        extra_worktrees: trees.len().saturating_sub(1),
        calls_per_min: calls as f64 / 5.0,
        cost: sessions.iter().map(|s| s.cost.total()).sum(),
    })
}

/// Two different sessions in the same working tree touched `path` inside the window:
/// the later edit landed on top of the earlier one.
fn conflict_text(path: &str, touches: &[(&Session, &antfarm_core::FileTouch)]) -> Option<String> {
    for (i, (later, lf)) in touches.iter().enumerate() {
        for (earlier, ef) in &touches[i + 1..] {
            if later.id == earlier.id || tree_key(later) != tree_key(earlier) {
                continue;
            }
            let (Some(lt), Some(et)) = (lf.at, ef.at) else {
                continue;
            };
            if (lt - et).num_seconds().abs() > RECENT_SECS {
                continue;
            }
            return Some(format!(
                "{} edited {} {} after {} did. Both sessions run in the same working tree, so the second edit landed on top of the first.",
                view::session_name(later),
                file_name(path),
                dur((lt - et).num_seconds().abs()),
                view::session_name(earlier),
            ));
        }
    }
    None
}

fn file_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use antfarm_core::{AgentId, FileTouch, Machine, SessionStatus};

    fn t(s: &str) -> Time {
        s.parse().unwrap()
    }

    fn session(id: &str, tree: &str, touches: &[(&str, &str, u64, u64)]) -> Session {
        let mut s = Session::new(SessionId::from(id), tree.into());
        s.name = Some(id.into());
        s.status = SessionStatus::Working;
        s.repo = Some("/w/app".into());
        s.worktree = Some(tree.into());
        s.branch = Some(if tree == "/w/app" {
            "main".into()
        } else {
            "fix".into()
        });
        for (path, at, added, removed) in touches {
            s.files_touched.push(FileTouch {
                path: (*path).into(),
                agent: AgentId::from(id),
                tool: "Edit".into(),
                at: Some(t(at)),
                added: *added,
                removed: *removed,
            });
        }
        s
    }

    fn snap(sessions: Vec<Session>) -> Snapshot {
        let mut m = Machine::default();
        for s in sessions {
            m.sessions.insert(s.id.clone(), s);
        }
        Snapshot {
            seq: 1,
            at: t("2026-11-03T12:00:00Z"),
            machine: m,
        }
    }

    #[test]
    fn same_tree_edits_conflict_and_worktrees_do_not() {
        let now = t("2026-11-03T12:00:00Z");
        let a = session(
            "auth",
            "/w/app",
            &[
                ("/w/app/src/session.ts", "2026-11-03T11:55:00Z", 112, 87),
                ("/w/app/db/m.sql", "2026-11-03T11:50:00Z", 41, 0),
            ],
        );
        let b = session(
            "limiter",
            "/w/app",
            &[
                ("/w/app/src/session.ts", "2026-11-03T11:58:00Z", 9, 2),
                ("/w/app/src/bucket.ts", "2026-11-03T11:59:00Z", 38, 4),
            ],
        );
        let c = session(
            "flaky",
            "/w/app-flaky",
            &[("/w/app-flaky/src/session.ts", "2026-11-03T11:59:30Z", 3, 1)],
        );
        let old = session(
            "old",
            "/w/app",
            &[("/w/app/src/session.ts", "2026-11-03T10:00:00Z", 1, 1)],
        );
        let s = snap(vec![a, b, c, old]);
        let v = project_view(&s, "/w/app", now).unwrap();
        assert_eq!(v.sessions.len(), 4);
        assert_eq!(v.extra_worktrees, 1);
        assert_eq!(v.conflicts, 1);
        let f = &v.files[0];
        assert_eq!(f.path, "/w/app/src/session.ts");
        let text = f.conflict.as_deref().unwrap();
        assert!(
            text.starts_with("limiter edited session.ts 3m00s after auth did"),
            "{text}"
        );
        assert_eq!(f.touches.len(), 2, "the 10:00 edit is outside the window");
        // The worktree's file of the same name is a different path in a different tree.
        assert!(
            v.files
                .iter()
                .any(|f| f.path == "/w/app-flaky/src/session.ts" && f.conflict.is_none())
        );
        let auth = v.sessions.iter().find(|x| x.name == "auth").unwrap();
        assert_eq!((auth.added, auth.removed), (153, 87));
        assert_eq!(auth.shares, ["session.ts"]);
        let flaky = v.sessions.iter().find(|x| x.name == "flaky").unwrap();
        assert!(flaky.worktree && flaky.shares.is_empty());
        assert!(
            v.branches
                .iter()
                .any(|b| b.branch == "main" && b.sessions == 3 && b.conflict)
        );
        assert!(v.branches.iter().any(|b| b.branch == "fix" && !b.conflict));
        assert_eq!(projects(&s).len(), 1);
    }
}
