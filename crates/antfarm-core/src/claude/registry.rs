//! Reads the live session registry, `~/.claude/sessions/<pid>.json`.

use std::path::{Path, PathBuf};

use super::files::RegistryEntry;

#[derive(Debug)]
pub struct RegistryFile {
    pub path: PathBuf,
    pub pid: u32,
    pub entry: Result<RegistryEntry, String>,
}

/// Lists every `<pid>.json` in the sessions directory. Other files, such as the
/// `<pid>.<hash>.key` socket tokens, are never opened.
pub fn scan(sessions_dir: &Path) -> std::io::Result<Vec<RegistryFile>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(sessions_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(pid) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let entry = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<RegistryEntry>(&b).map_err(|e| e.to_string()));
        out.push(RegistryFile { path, pid, entry });
    }
    out.sort_by_key(|f| f.pid);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_only_pid_json_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("748.json"),
            r#"{"pid":748,"sessionId":"s1","status":"busy"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("12.json"), "{not json").unwrap();
        std::fs::write(dir.path().join("748.abc.key"), "secret").unwrap();
        std::fs::write(dir.path().join("notes.json"), "{}").unwrap();
        let files = scan(dir.path()).unwrap();
        assert_eq!(files.iter().map(|f| f.pid).collect::<Vec<_>>(), [12, 748]);
        assert!(files[0].entry.is_err());
        assert_eq!(files[1].entry.as_ref().unwrap().session_id, "s1");
        assert!(scan(&dir.path().join("missing")).unwrap().is_empty());
    }
}
