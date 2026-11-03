//! Which repository and working tree a directory belongs to, read from `.git` without
//! running git.

use std::path::{Path, PathBuf};

/// `(repository root, working tree root)` for `dir`. A git worktree's `.git` is a file
/// pointing into the main repository's `.git/worktrees/<name>`, so both roots differ; in
/// the main checkout they are the same.
pub fn repo_of(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    for anc in dir.ancestors() {
        let git = anc.join(".git");
        if git.is_dir() {
            return Some((anc.to_path_buf(), anc.to_path_buf()));
        }
        if git.is_file() {
            let text = std::fs::read_to_string(&git).ok()?;
            let target = text
                .lines()
                .find_map(|l| l.strip_prefix("gitdir:"))
                .map(str::trim)?;
            let target = if Path::new(target).is_absolute() {
                PathBuf::from(target)
            } else {
                anc.join(target)
            };
            let s = target.to_string_lossy();
            let repo = s
                .find("/.git/worktrees/")
                .map(|i| PathBuf::from(&s[..i]))
                .unwrap_or_else(|| anc.to_path_buf());
            return Some((repo, anc.to_path_buf()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_checkout_and_worktree() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("app");
        std::fs::create_dir_all(repo.join(".git/worktrees/fix")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        let wt = root.path().join("app-fix");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", repo.join(".git/worktrees/fix").display()),
        )
        .unwrap();
        assert_eq!(
            repo_of(&repo.join("src/deep")),
            Some((repo.clone(), repo.clone()))
        );
        assert_eq!(repo_of(&wt.join("src")), Some((repo.clone(), wt.clone())));
        assert_eq!(repo_of(Path::new("/")), None);
    }
}
