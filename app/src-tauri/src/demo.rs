//! The tutorial's sample sessions, read by the same live source as real ones.
//!
//! The made-up transcripts in `fixtures/demo` are bundled into the app. Starting the
//! demo writes them into a private Claude Code folder with their times moved to the
//! present: everything recorded before the manifest's `liveFrom` at once, and the rest
//! at the moment it was recorded, so the sessions keep moving while the tutorial runs.
//! A [`ClaudeCodeSource`] reads that folder the way it reads `~/.claude`, with a process
//! probe that reports the sample sessions as running.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use antfarm_core::claude::files::RegistryEntry;
use antfarm_core::claude::locate::slug_guess;
use antfarm_core::claude::{ClaudeCodeSource, SourceConfig, Wants};
use antfarm_core::input::Input;
use antfarm_core::proc::ProcessProbe;
use antfarm_core::source::{Clock, Handle, Hub, Source, SourceError};
use antfarm_core::{Paths, SessionId, SourceApp, Time};
use include_dir::{Dir, include_dir};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

static SAMPLES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../fixtures/demo");

/// Stands in for a background command's output folder in the sample transcripts.
const TASKS: &str = "{{TASKS}}";

/// The sample sessions' files, kept up to date while the tutorial runs. Dropping it
/// stops the writing and removes the folder.
pub struct Samples {
    player: JoinHandle<()>,
    _home: tempfile::TempDir,
}

impl Drop for Samples {
    fn drop(&mut self) {
        self.player.abort();
    }
}

/// Writes the samples as of `now` and starts a hub reading them. Stop the hub, then
/// drop the samples, when the tutorial ends.
pub fn start(now: Time) -> std::io::Result<(Handle, Wants, Samples)> {
    {
        let home = tempfile::Builder::new()
            .prefix("antfarm-tutorial-")
            .tempdir()?;
        let script = Script::from_samples(&SAMPLES, home.path(), now)?;
        for w in script.writes.iter().take_while(|w| w.at <= now) {
            w.apply(home.path())?;
        }
        let later: Vec<Write> = script.writes.into_iter().filter(|w| w.at > now).collect();
        let player = tokio::spawn(play(home.path().to_path_buf(), later, now));
        let mut config = SourceConfig::new(Paths {
            claude_home: home.path().to_path_buf(),
        });
        config.poll_interval = Duration::from_millis(500);
        let inner = ClaudeCodeSource::with_probe(
            config,
            Box::new(SampleProbe {
                running: script.running,
            }),
        );
        let wants = inner.wants();
        let source = SampleSource {
            inner,
            sources: script.sources,
        };
        let handle = Hub::spawn(Box::new(source), Clock::Wall);
        Ok((
            handle,
            wants,
            Samples {
                player,
                _home: home,
            },
        ))
    }
}

/// Appends to or replaces a file in the private folder at a given time.
#[derive(Debug, Clone, PartialEq)]
struct Write {
    at: Time,
    /// Relative to the private folder.
    path: PathBuf,
    text: String,
    replace: bool,
}

impl Write {
    fn apply(&self, home: &Path) -> std::io::Result<()> {
        let path = home.join(&self.path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if self.replace {
            return std::fs::write(&path, &self.text);
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        // One write per line, so the tailer never sees half of one.
        f.write_all(self.text.as_bytes())
    }
}

/// Everything the demo writes, in time order, plus what the probe and source need.
#[derive(Debug, Default)]
struct Script {
    writes: Vec<Write>,
    /// Sample sessions whose process counts as running.
    running: HashSet<u32>,
    /// The app each sample session runs in, as the process tree would show it.
    sources: HashMap<SessionId, SourceApp>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    live_from: Time,
    #[serde(default)]
    exited: Vec<u32>,
}

#[derive(serde::Deserialize)]
struct Chunk {
    at: Time,
    text: String,
}

fn bad(what: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_string())
}

fn text<'a>(dir: &'a Dir, path: &str) -> std::io::Result<&'a str> {
    dir.get_file(path)
        .and_then(|f| f.contents_utf8())
        .ok_or_else(|| bad(format!("sample file {path} is missing")))
}

impl Script {
    /// Lays the samples out as Claude Code would, shifted so `liveFrom` falls on `now`.
    fn from_samples(samples: &Dir, home: &Path, now: Time) -> std::io::Result<Self> {
        let manifest: Manifest = serde_json::from_str(text(samples, "demo.json")?).map_err(bad)?;
        let shift = now - manifest.live_from;
        let tasks = home.join("tasks");
        let tasks_json = serde_json::to_string(&tasks.display().to_string()).map_err(bad)?;
        let tasks_json = &tasks_json[1..tasks_json.len() - 1];
        let mut script = Script::default();
        for session in samples.dirs() {
            let name = session.path().display().to_string();
            let reg_text = text(samples, &format!("{name}/registry.json"))?;
            let mut reg: serde_json::Value = serde_json::from_str(reg_text).map_err(bad)?;
            let entry: RegistryEntry = serde_json::from_value(reg.clone()).map_err(bad)?;
            let id = SessionId::from(entry.session_id.as_str());
            let project =
                PathBuf::from("projects").join(slug_guess(entry.cwd.as_deref().unwrap_or("/")));
            if let Some(ms) = entry.status_updated_at {
                reg["statusUpdatedAt"] = (ms + shift.num_milliseconds()).into();
            }
            let earliest = now - chrono::Duration::days(1);
            script.writes.push(Write {
                at: earliest,
                path: PathBuf::from("sessions").join(format!("{}.json", entry.pid)),
                text: reg.to_string(),
                replace: true,
            });
            if !manifest.exited.contains(&entry.pid) {
                script.running.insert(entry.pid);
            }
            if let Some(f) = samples.get_file(format!("{name}/source.json")) {
                let src: SourceApp = serde_json::from_slice(f.contents()).map_err(bad)?;
                script.sources.insert(id.clone(), src);
            }
            let main = project.join(format!("{}.jsonl", id.as_str()));
            script.lines(
                text(samples, &format!("{name}/main.jsonl"))?,
                &main,
                None,
                shift,
                tasks_json,
            )?;
            let subs = project.join(id.as_str()).join("subagents");
            if let Some(dir) = samples.get_dir(format!("{name}/subagents")) {
                for f in dir.files() {
                    let Some(file) = f.path().file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    let Some(stem) = file.strip_suffix(".jsonl") else {
                        continue;
                    };
                    let meta = samples
                        .get_file(format!("{name}/subagents/{stem}.meta.json"))
                        .and_then(|m| m.contents_utf8())
                        .map(|m| (subs.join(format!("{stem}.meta.json")), m.to_owned()));
                    let lines = f.contents_utf8().ok_or_else(|| bad(file))?;
                    script.lines(lines, &subs.join(file), meta, shift, tasks_json)?;
                }
            }
            if let Some(dir) = samples.get_dir(format!("{name}/tasks")) {
                for f in dir.files() {
                    let Some(task) = f
                        .path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_suffix(".jsonl"))
                    else {
                        continue;
                    };
                    for line in f.contents_utf8().unwrap_or_default().lines() {
                        let c: Chunk = serde_json::from_str(line).map_err(bad)?;
                        script.writes.push(Write {
                            at: c.at + shift,
                            path: PathBuf::from("tasks").join(format!("{task}.output")),
                            text: c.text,
                            replace: false,
                        });
                    }
                }
            }
        }
        // A stable sort keeps each file's lines, and a meta before its transcript, in order.
        script.writes.sort_by_key(|w| w.at);
        Ok(script)
    }

    /// One transcript's lines with their times shifted. A sub-agent's meta goes just
    /// before its first line, as Claude Code writes it when the agent starts.
    fn lines(
        &mut self,
        jsonl: &str,
        path: &Path,
        meta: Option<(PathBuf, String)>,
        shift: chrono::Duration,
        tasks: &str,
    ) -> std::io::Result<()> {
        let mut meta = meta;
        let mut last: Option<Time> = None;
        for raw in jsonl.lines().filter(|l| !l.trim().is_empty()) {
            let mut v: serde_json::Value = serde_json::from_str(raw).map_err(bad)?;
            if let Some(t) = v
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(|t| t.parse::<Time>().ok())
            {
                let t = t + shift;
                v["timestamp"] = t
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                    .into();
                last = Some(t);
            }
            let at =
                last.ok_or_else(|| bad(format!("{} starts without a time", path.display())))?;
            if let Some((mpath, mtext)) = meta.take() {
                self.writes.push(Write {
                    at,
                    path: mpath,
                    text: mtext,
                    replace: true,
                });
            }
            self.writes.push(Write {
                at,
                path: path.to_path_buf(),
                text: format!("{}\n", v.to_string().replace(TASKS, tasks)),
                replace: false,
            });
        }
        Ok(())
    }
}

/// Writes the rest of the script as its moments arrive.
async fn play(home: PathBuf, writes: Vec<Write>, start: Time) {
    let begun = tokio::time::Instant::now();
    for w in writes {
        let due = (w.at - start).to_std().unwrap_or_default();
        tokio::time::sleep_until(begun + due).await;
        if let Err(e) = w.apply(&home) {
            eprintln!("antfarm: tutorial sample: {e}");
        }
    }
}

/// Every sample session is running except those the manifest says have exited. There
/// are no real processes, so there is no ancestry to find an app from.
struct SampleProbe {
    running: HashSet<u32>,
}

impl ProcessProbe for SampleProbe {
    fn alive(&mut self, pid: u32, _started_at_ms: Option<i64>) -> bool {
        self.running.contains(&pid)
    }
}

/// The live source, plus the app each sample session runs in, sent once the session
/// has appeared, since the probe can't find it from a process tree.
struct SampleSource {
    inner: ClaudeCodeSource,
    sources: HashMap<SessionId, SourceApp>,
}

impl Source for SampleSource {
    fn name(&self) -> &'static str {
        "tutorial"
    }

    fn start(
        self: Box<Self>,
        sink: mpsc::Sender<Input>,
        stop: CancellationToken,
    ) -> JoinHandle<Result<(), SourceError>> {
        let (tx, mut rx) = mpsc::channel(256);
        let inner = Box::new(self.inner).start(tx, stop);
        let mut sources = self.sources;
        tokio::spawn(async move {
            while let Some(input) = rx.recv().await {
                let found = match &input {
                    Input::Registry { entry, .. } => sources
                        .remove_entry(entry.session_id.as_str())
                        .map(|(session, source)| Input::SessionSource { session, source }),
                    _ => None,
                };
                if sink.send(input).await.is_err() {
                    break;
                }
                if let Some(f) = found
                    && sink.send(f).await.is_err()
                {
                    break;
                }
            }
            inner.await.map_err(|e| SourceError::Other(e.to_string()))?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Time {
        s.parse().unwrap()
    }

    #[test]
    fn moves_the_samples_to_the_present() {
        let home = Path::new("/h");
        let now = at("2030-01-01T12:00:00Z");
        let s = Script::from_samples(&SAMPLES, home, now).unwrap();
        // Something was recorded before the tutorial starts, and something after.
        assert!(s.writes.iter().any(|w| w.at < now));
        assert!(
            s.writes
                .iter()
                .any(|w| w.at > now + chrono::Duration::seconds(60))
        );
        // In order, and every transcript line carries its new time.
        assert!(s.writes.windows(2).all(|w| w[0].at <= w[1].at));
        for w in s
            .writes
            .iter()
            .filter(|w| w.path.extension().is_some_and(|e| e == "jsonl"))
        {
            let v: serde_json::Value = serde_json::from_str(w.text.trim_end()).unwrap();
            if let Some(t) = v["timestamp"].as_str() {
                assert_eq!(t.parse::<Time>().unwrap(), w.at);
            }
        }
        // The question and the permission prompt are both waiting from the start.
        let reg: Vec<&Write> = s
            .writes
            .iter()
            .filter(|w| w.path.starts_with("sessions"))
            .collect();
        assert!(reg.iter().all(|w| w.at < now));
        assert_eq!(
            reg.iter()
                .filter(|w| w.text.contains("\"waiting\""))
                .count(),
            2
        );
    }

    #[test]
    fn lays_files_out_as_claude_code_does() {
        let s =
            Script::from_samples(&SAMPLES, Path::new("/h"), at("2030-01-01T12:00:00Z")).unwrap();
        let paths: HashSet<String> = s
            .writes
            .iter()
            .map(|w| w.path.display().to_string())
            .collect();
        assert!(paths.contains("projects/-Users-demo-code-storefront/demo-apple-pay.jsonl"));
        assert!(paths.contains(
            "projects/-Users-demo-code-storefront/demo-apple-pay/subagents/agent-a5b7e2c91d04f6a28.jsonl"
        ));
        assert!(paths.contains(
            "projects/-Users-demo-code-storefront/demo-apple-pay/subagents/agent-a5b7e2c91d04f6a28.meta.json"
        ));
        assert!(paths.contains("tasks/bk7f2a.output"));
        // A sub-agent's meta is written just before its transcript starts.
        let first = |p: &str| {
            s.writes
                .iter()
                .position(|w| w.path.display().to_string().ends_with(p))
                .unwrap()
        };
        assert!(
            first("agent-a4d0e8b2c6f19a37e.meta.json") < first("agent-a4d0e8b2c6f19a37e.jsonl")
        );
        // Background output points into the private folder.
        assert!(
            s.writes
                .iter()
                .any(|w| w.text.contains("/h/tasks/bk7f2a.output"))
        );
        assert!(!s.writes.iter().any(|w| w.text.contains(TASKS)));
        assert!(s.running.contains(&41001) && !s.running.contains(&41004));
        assert_eq!(
            s.sources["demo-apple-pay"].tmux_pane.as_deref(),
            Some("shop:1.0")
        );
    }

    /// The samples run through the real live source: both waits show up, the sessions
    /// carry their apps, and the exited one is exited.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_live_source_reads_the_samples() {
        let (handle, _wants, _samples) = start(chrono::Utc::now()).unwrap();
        let mut rx = handle.snapshots();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let ok = |snap: &antfarm_core::Snapshot| {
            let s = &snap.machine.sessions;
            s.len() == 5
                && s.values()
                    .filter(|s| matches!(s.status, antfarm_core::SessionStatus::Waiting(_)))
                    .count()
                    == 2
                && s.get("demo-apple-pay")
                    .is_some_and(|s| s.source.is_some() && s.agents.len() > 3)
                && s.get("demo-lint")
                    .is_some_and(|s| s.status == antfarm_core::SessionStatus::Exited)
        };
        loop {
            if ok(&rx.borrow_and_update()) {
                break;
            }
            assert!(
                tokio::time::timeout_at(deadline, rx.changed())
                    .await
                    .is_ok(),
                "the samples never all appeared: {:#?}",
                rx.borrow()
                    .machine
                    .sessions
                    .values()
                    .map(|s| (s.id.clone(), s.status, s.agents.len(), s.source.clone()))
                    .collect::<Vec<_>>()
            );
        }
        handle.stop();
    }
}
