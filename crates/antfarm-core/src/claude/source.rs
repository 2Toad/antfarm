//! The live Claude Code source: registry, transcripts, sub-agents, and workflows.
//!
//! One blocking thread owns every tailer and the watch set. Each pass scans the
//! registry, attaches to the transcripts of live sessions, polls every tracked file, and
//! rescans sub-agent and workflow directories at most once per [`DIR_SCAN_EVERY`]. A
//! pass runs on a change hint or after `poll_interval`, whichever comes first.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::cost::cost_anchor;
use super::files::{RegistryEntry, SubagentMeta, WorkflowFile};
use super::locate::{Locator, SessionDirs};
use super::parse::parse_batch;
use super::registry;
use crate::Paths;
use crate::input::{AgentRef, Input};
use crate::model::{AgentId, ParseStats, SessionId, Time};
use crate::proc::{ProcessProbe, SysinfoProbe};
use crate::source::{Source, SourceError};
use crate::tail::watcher::WatchSet;
use crate::tail::{TailConfig, Tailer};

/// Sub-agent and workflow directories are re-listed at most this often.
pub const DIR_SCAN_EVERY: Duration = Duration::from_secs(1);
/// Most lines sent to the engine in one input.
const BATCH_LINES: usize = 2000;

#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub paths: Paths,
    /// Longest wait between passes when nothing signals a change.
    pub poll_interval: Duration,
    /// Skip the native watcher and rely on polling alone.
    pub poll_only: bool,
    pub main_tail: u64,
    pub sub_tail: u64,
    /// Sub-agent transcripts older than this are not read; their meta still is.
    pub sub_recent: Duration,
    /// Workflow files older than this at startup are not read.
    pub workflow_recent: Duration,
    /// How far back from the end to look for the last cost-state line.
    pub cost_scan: u64,
}

impl SourceConfig {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            poll_interval: Duration::from_secs(2),
            poll_only: false,
            main_tail: 16 << 20,
            sub_tail: 4 << 20,
            sub_recent: Duration::from_secs(3600),
            workflow_recent: Duration::from_secs(6 * 3600),
            cost_scan: 128 << 20,
        }
    }
}

/// What the source found, for `antfarm doctor`.
#[derive(Debug, Clone, Default)]
pub struct SourceStats {
    pub backend: String,
    pub passes: u64,
    pub registry: Vec<RegistryRow>,
    pub transcripts: Vec<(SessionId, Option<PathBuf>)>,
    pub session_dirs: Vec<(SessionId, SessionDirs)>,
    pub files_tailed: usize,
    pub bytes_read: u64,
    pub parse: ParseStats,
    pub oversized: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RegistryRow {
    pub path: PathBuf,
    pub pid: u32,
    pub entry: Result<RegistryEntry, String>,
    pub alive: bool,
}

pub struct ClaudeCodeSource {
    config: SourceConfig,
    probe: Box<dyn ProcessProbe>,
    stats: Arc<Mutex<SourceStats>>,
}

impl ClaudeCodeSource {
    pub fn new(config: SourceConfig) -> Self {
        Self::with_probe(config, Box::new(SysinfoProbe::default()))
    }

    pub fn with_probe(config: SourceConfig, probe: Box<dyn ProcessProbe>) -> Self {
        Self {
            config,
            probe,
            stats: Arc::default(),
        }
    }

    /// Shared view of what the source has found so far.
    pub fn stats(&self) -> Arc<Mutex<SourceStats>> {
        self.stats.clone()
    }
}

impl Source for ClaudeCodeSource {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn start(
        self: Box<Self>,
        sink: mpsc::Sender<Input>,
        stop: CancellationToken,
    ) -> JoinHandle<Result<(), SourceError>> {
        tokio::task::spawn_blocking(move || {
            let mut w = Worker::new(*self, sink);
            w.run(&stop)
        })
    }
}

struct Track {
    tailer: Tailer,
    agent: AgentRef,
}

#[derive(Default)]
struct SessionTrack {
    cwd: Option<String>,
    alive: bool,
    main: Option<Track>,
    subs: HashMap<AgentId, Track>,
    seen: HashMap<PathBuf, SystemTime>,
    dirs: SessionDirs,
    last_dir_scan: Option<Instant>,
}

struct Worker {
    config: SourceConfig,
    probe: Box<dyn ProcessProbe>,
    stats: Arc<Mutex<SourceStats>>,
    sink: mpsc::Sender<Input>,
    locator: Locator,
    watch: WatchSet,
    registry: HashMap<u32, (RegistryEntry, bool)>,
    sessions: HashMap<SessionId, SessionTrack>,
    started: SystemTime,
}

/// The source stops quietly when the hub is gone.
struct Closed;

impl Worker {
    fn new(src: ClaudeCodeSource, sink: mpsc::Sender<Input>) -> Self {
        let watch = WatchSet::new(src.config.poll_only);
        src.stats.lock().unwrap().backend = watch.backend().to_owned();
        Self {
            locator: Locator::new(src.config.paths.projects_dir()),
            config: src.config,
            probe: src.probe,
            stats: src.stats,
            sink,
            watch,
            registry: HashMap::new(),
            sessions: HashMap::new(),
            started: SystemTime::now(),
        }
    }

    fn run(&mut self, stop: &CancellationToken) -> Result<(), SourceError> {
        let sessions_dir = self.config.paths.sessions_dir();
        let interval = if self.watch.is_native() {
            self.config.poll_interval
        } else {
            self.config.poll_interval.min(Duration::from_millis(500))
        };
        while !stop.is_cancelled() {
            self.watch.watch(&sessions_dir);
            if self.pass().is_err() {
                return Ok(());
            }
            self.stats.lock().unwrap().passes += 1;
            // Wait in short slices so a stop request is noticed promptly.
            let deadline = Instant::now() + interval;
            while !stop.is_cancelled() {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || self.watch.wait(left.min(Duration::from_millis(200))) {
                    break;
                }
            }
        }
        Ok(())
    }

    fn send(&self, input: Input) -> Result<(), Closed> {
        self.sink.blocking_send(input).map_err(|_| Closed)
    }

    fn pass(&mut self) -> Result<(), Closed> {
        self.scan_registry()?;
        let ids: Vec<SessionId> = self.sessions.keys().cloned().collect();
        for id in ids {
            self.poll_session(&id)?;
        }
        Ok(())
    }

    fn scan_registry(&mut self) -> Result<(), Closed> {
        let files = registry::scan(&self.config.paths.sessions_dir()).unwrap_or_default();
        let mut present = HashSet::new();
        let mut rows = Vec::new();
        for f in files {
            present.insert(f.pid);
            let alive = match &f.entry {
                Ok(e) => self.probe.alive(f.pid, e.started_at),
                Err(_) => false,
            };
            rows.push(RegistryRow {
                path: f.path.clone(),
                pid: f.pid,
                entry: f.entry.clone(),
                alive,
            });
            let Ok(entry) = f.entry else { continue };
            if entry.session_id.is_empty() {
                continue;
            }
            let changed = self
                .registry
                .get(&f.pid)
                .is_none_or(|(e, a)| *e != entry || *a != alive);
            if changed {
                self.registry.insert(f.pid, (entry.clone(), alive));
                let sid = SessionId::from(entry.session_id.as_str());
                let t = self.sessions.entry(sid).or_default();
                t.cwd = entry.cwd.clone();
                t.alive = alive;
                self.send(Input::Registry { entry, alive })?;
            }
        }
        let gone: Vec<u32> = self
            .registry
            .keys()
            .copied()
            .filter(|p| !present.contains(p))
            .collect();
        for pid in gone {
            if let Some((e, _)) = self.registry.remove(&pid) {
                if let Some(t) = self.sessions.get_mut(e.session_id.as_str()) {
                    t.alive = false;
                }
                self.send(Input::RegistryGone { pid })?;
            }
        }
        self.stats.lock().unwrap().registry = rows;
        Ok(())
    }

    fn poll_session(&mut self, id: &SessionId) -> Result<(), Closed> {
        let Some(t) = self.sessions.get(id) else {
            return Ok(());
        };
        // Exited sessions are drained once more, then left alone.
        if !t.alive && t.main.is_none() {
            return Ok(());
        }
        if t.main.is_none() {
            self.attach_main(id)?;
        }
        if let Some(track) = self.sessions.get_mut(id).and_then(|t| t.main.take()) {
            let track = self.drain(id, track)?;
            self.sessions.get_mut(id).unwrap().main = Some(track);
        }
        self.scan_dirs(id)?;
        let subs: Vec<AgentId> = self.sessions[id].subs.keys().cloned().collect();
        for a in subs {
            if let Some(track) = self.sessions.get_mut(id).and_then(|t| t.subs.remove(&a)) {
                let track = self.drain(id, track)?;
                self.sessions.get_mut(id).unwrap().subs.insert(a, track);
            }
        }
        let t = self.sessions.get_mut(id).unwrap();
        if !t.alive {
            t.main = None;
            t.subs.clear();
        }
        self.refresh_counts();
        Ok(())
    }

    fn attach_main(&mut self, id: &SessionId) -> Result<(), Closed> {
        let cwd = self.sessions[id].cwd.clone();
        let path = self.locator.transcript(id.as_str(), cwd.as_deref());
        self.record_transcript(id, path.clone());
        let Some(path) = path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            self.watch.watch(dir);
        }
        let config = TailConfig {
            max_tail: self.config.main_tail,
            ..TailConfig::default()
        };
        let tailer = match Tailer::attach(&path, config) {
            Ok(t) => t,
            Err(e) => {
                self.error(format!("attach {}: {e}", path.display()));
                return Ok(());
            }
        };
        self.send(Input::Attached {
            session: id.clone(),
            agent: AgentRef::Root,
            path: path.clone(),
            truncated_before: tailer.truncated_before(),
        })?;
        if let Ok(Some((usd, offset, at))) = cost_anchor(&path, self.config.cost_scan) {
            self.send(Input::CostAnchor {
                session: id.clone(),
                usd,
                offset,
                at,
            })?;
        }
        self.sessions.get_mut(id).unwrap().main = Some(Track {
            tailer,
            agent: AgentRef::Root,
        });
        Ok(())
    }

    /// Polls one tailer and sends what it read.
    fn drain(&mut self, id: &SessionId, mut track: Track) -> Result<Track, Closed> {
        let poll = match track.tailer.poll() {
            Ok(p) => p,
            Err(e) => {
                self.error(format!("read {}: {e}", track.tailer.path().display()));
                return Ok(track);
            }
        };
        if poll.truncated {
            self.send(Input::FileTruncated {
                session: id.clone(),
                agent: track.agent.clone(),
            })?;
        }
        if poll.lines.is_empty() {
            return Ok(track);
        }
        let mtime = mtime(track.tailer.path());
        let mut stats = ParseStats::default();
        let bytes: u64 = poll.lines.iter().map(|l| l.bytes.len() as u64 + 1).sum();
        let mut lines = poll.lines;
        while !lines.is_empty() {
            let rest = lines.split_off(lines.len().min(BATCH_LINES));
            let (good, errors) = parse_batch(lines, &mut stats);
            self.send(Input::Lines {
                session: id.clone(),
                agent: track.agent.clone(),
                lines: good,
                errors,
                end_offset: poll.end_offset,
                mtime,
            })?;
            lines = rest;
        }
        let mut s = self.stats.lock().unwrap();
        s.bytes_read += bytes;
        merge_stats(&mut s.parse, &stats);
        s.oversized = s.oversized.max(track.tailer.oversized());
        drop(s);
        Ok(track)
    }

    fn scan_dirs(&mut self, id: &SessionId) -> Result<(), Closed> {
        let t = &self.sessions[id];
        if t.last_dir_scan
            .is_some_and(|at| at.elapsed() < DIR_SCAN_EVERY)
        {
            return Ok(());
        }
        let dirs = self.locator.session_dirs(id.as_str());
        for d in dirs.subagents.iter().chain(&dirs.workflows) {
            self.watch.watch(d);
        }
        {
            let t = self.sessions.get_mut(id).unwrap();
            t.last_dir_scan = Some(Instant::now());
            t.dirs = dirs.clone();
        }
        self.record_dirs(id, &dirs);
        for dir in &dirs.subagents {
            self.scan_subagents(id, dir)?;
        }
        for dir in &dirs.workflows {
            self.scan_workflows(id, dir)?;
        }
        Ok(())
    }

    fn scan_subagents(&mut self, id: &SessionId, dir: &Path) -> Result<(), Closed> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Ok(());
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        // Metas first, so a sub-agent's type and parent are known before its lines.
        for path in entries.iter().filter(|p| ends_with(p, ".meta.json")) {
            let Some(m) = mtime_sys(path) else { continue };
            if self.sessions[id].seen.get(path) == Some(&m) {
                continue;
            }
            let Some(agent) = agent_id(path, ".meta.json") else {
                continue;
            };
            let transcript = path.with_file_name(format!("agent-{agent}.jsonl"));
            let meta: Option<SubagentMeta> = std::fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            if let Some(meta) = meta {
                self.send(Input::SubagentMeta {
                    session: id.clone(),
                    agent: AgentId::from(agent),
                    meta,
                    transcript_mtime: mtime(&transcript),
                })?;
            }
            self.sessions
                .get_mut(id)
                .unwrap()
                .seen
                .insert(path.clone(), m);
        }
        let recent = self.config.sub_recent;
        for path in entries.iter().filter(|p| ends_with(p, ".jsonl")) {
            let Some(agent) = agent_id(path, ".jsonl").map(AgentId::from) else {
                continue;
            };
            if self.sessions[id].subs.contains_key(&agent) {
                continue;
            }
            let fresh = mtime_sys(path)
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age <= recent);
            if !fresh {
                continue;
            }
            let config = TailConfig {
                max_tail: self.config.sub_tail,
                ..TailConfig::default()
            };
            let Ok(tailer) = Tailer::attach(path, config) else {
                continue;
            };
            let agent_ref = AgentRef::Sub(agent.clone());
            self.send(Input::Attached {
                session: id.clone(),
                agent: agent_ref.clone(),
                path: path.clone(),
                truncated_before: tailer.truncated_before(),
            })?;
            self.sessions.get_mut(id).unwrap().subs.insert(
                agent,
                Track {
                    tailer,
                    agent: agent_ref,
                },
            );
        }
        Ok(())
    }

    fn scan_workflows(&mut self, id: &SessionId, dir: &Path) -> Result<(), Closed> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Ok(());
        };
        let horizon = self.started.checked_sub(self.config.workflow_recent);
        for path in rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| ends_with(p, ".json"))
        {
            let Some(m) = mtime_sys(&path) else { continue };
            if horizon.is_some_and(|h| m < h) || self.sessions[id].seen.get(&path) == Some(&m) {
                continue;
            }
            if let Some(file) = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<WorkflowFile>(&b).ok())
            {
                self.send(Input::Workflow {
                    session: id.clone(),
                    file,
                })?;
            }
            self.sessions.get_mut(id).unwrap().seen.insert(path, m);
        }
        Ok(())
    }

    fn record_transcript(&self, id: &SessionId, path: Option<PathBuf>) {
        let mut s = self.stats.lock().unwrap();
        s.transcripts.retain(|(i, _)| i != id);
        s.transcripts.push((id.clone(), path));
    }

    fn record_dirs(&self, id: &SessionId, dirs: &SessionDirs) {
        let mut s = self.stats.lock().unwrap();
        s.session_dirs.retain(|(i, _)| i != id);
        s.session_dirs.push((id.clone(), dirs.clone()));
    }

    fn refresh_counts(&self) {
        let n = self
            .sessions
            .values()
            .map(|t| usize::from(t.main.is_some()) + t.subs.len())
            .sum();
        self.stats.lock().unwrap().files_tailed = n;
    }

    fn error(&self, msg: String) {
        let mut s = self.stats.lock().unwrap();
        if s.errors.len() < 100 {
            s.errors.push(msg);
        }
    }
}

fn merge_stats(into: &mut ParseStats, from: &ParseStats) {
    into.lines += from.lines;
    into.bytes += from.bytes;
    into.parsed += from.parsed;
    into.errors += from.errors;
    if into.first_error.is_none() {
        into.first_error = from.first_error.clone();
    }
    for (k, v) in &from.ignored_types {
        *into.ignored_types.entry(k.clone()).or_default() += v;
    }
    for (k, v) in &from.versions {
        *into.versions.entry(k.clone()).or_default() += v;
    }
}

fn ends_with(p: &Path, suffix: &str) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(suffix) && (suffix != ".jsonl" || !n.ends_with(".meta.jsonl")))
}

/// `agent-<id><suffix>` to `<id>`.
fn agent_id<'a>(p: &'a Path, suffix: &str) -> Option<&'a str> {
    p.file_name()?
        .to_str()?
        .strip_prefix("agent-")?
        .strip_suffix(suffix)
}

fn mtime_sys(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn mtime(p: &Path) -> Option<Time> {
    mtime_sys(p).map(DateTime::<Utc>::from)
}
