//! Plays a recorded session back through the engine, at a chosen speed.
//!
//! The main transcript and every sub-agent transcript beside it are merged into one
//! timeline by line timestamp. Each sub-agent's meta is sent just before its first line,
//! and workflow files at the end. The source sends its own `Tick`s at line time, so
//! statuses age as they did when the session ran.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::claude::cost::cost_anchor;
use crate::claude::files::{SubagentMeta, WorkflowFile};
use crate::claude::parse::{ParsedLine, parse_batch};
use crate::input::{AgentRef, Input};
use crate::model::{AgentId, ParseStats, SessionId, Time};
use crate::source::{Source, SourceError};
use crate::tail::{TailConfig, Tailer};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Speed {
    /// No waiting at all.
    Max,
    /// Recorded time divided by this factor.
    Times(f64),
}

#[derive(Debug, Clone)]
pub struct ReplayConfig {
    pub main: PathBuf,
    pub speed: Speed,
    /// Longest real pause between two lines, so idle stretches do not stall a replay.
    pub max_gap: Duration,
    pub max_tail: u64,
    /// Session id to report; defaults to the transcript's file stem.
    pub session: Option<String>,
}

impl ReplayConfig {
    pub fn new(main: impl Into<PathBuf>) -> Self {
        Self {
            main: main.into(),
            speed: Speed::Max,
            max_gap: Duration::from_secs(3),
            max_tail: 16 << 20,
            session: None,
        }
    }
}

pub struct ReplaySource {
    config: ReplayConfig,
}

impl ReplaySource {
    pub fn new(config: ReplayConfig) -> Self {
        Self { config }
    }
}

/// The directories beside a transcript: `<stem>/subagents` as Claude Code lays them out,
/// or `subagents` next to it as fixtures do.
pub fn side_dirs(main: &Path) -> (PathBuf, PathBuf) {
    let dir = main.parent().unwrap_or(Path::new("."));
    let stem = main
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let nested = dir.join(stem);
    let base = if nested.join("subagents").is_dir() || nested.join("workflows").is_dir() {
        nested
    } else {
        dir.to_path_buf()
    };
    (base.join("subagents"), base.join("workflows"))
}

struct Track {
    agent: AgentRef,
    path: PathBuf,
    truncated_before: Option<u64>,
    lines: Vec<ParsedLine>,
    errors: Vec<crate::claude::parse::ParseError>,
    meta: Option<SubagentMeta>,
}

/// One step of the merged timeline.
struct Step {
    at: Option<Time>,
    track: usize,
    line: usize,
}

impl Source for ReplaySource {
    fn name(&self) -> &'static str {
        "replay"
    }

    fn start(
        self: Box<Self>,
        sink: mpsc::Sender<Input>,
        stop: CancellationToken,
    ) -> JoinHandle<Result<(), SourceError>> {
        tokio::spawn(async move {
            let c = self.config;
            let session = SessionId::from(c.session.clone().unwrap_or_else(|| {
                c.main
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("replay")
                    .to_owned()
            }));
            let main = c.main.clone();
            let max_tail = c.max_tail;
            let (tracks, workflows) = tokio::task::spawn_blocking(move || load(&main, max_tail))
                .await
                .map_err(|e| SourceError::Other(e.to_string()))??;
            play(&c, &session, tracks, workflows, &sink, &stop).await;
            Ok(())
        })
    }
}

fn read_track(path: &Path, agent: AgentRef, max_tail: u64) -> std::io::Result<Track> {
    let mut tailer = Tailer::attach(
        path,
        TailConfig {
            max_tail,
            ..TailConfig::default()
        },
    )?;
    let poll = tailer.poll()?;
    let (lines, errors) = parse_batch(poll.lines, &mut ParseStats::default());
    Ok(Track {
        agent,
        path: path.to_path_buf(),
        truncated_before: tailer.truncated_before(),
        lines,
        errors,
        meta: None,
    })
}

fn load(main: &Path, max_tail: u64) -> Result<(Vec<Track>, Vec<WorkflowFile>), SourceError> {
    let mut tracks = vec![read_track(main, AgentRef::Root, max_tail)?];
    let (subs, wfs) = side_dirs(main);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&subs)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for path in files {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id) = name
            .strip_prefix("agent-")
            .and_then(|n| n.strip_suffix(".jsonl"))
        else {
            continue;
        };
        let mut t = read_track(&path, AgentRef::Sub(AgentId::from(id)), max_tail)?;
        t.meta = std::fs::read(path.with_file_name(format!("agent-{id}.meta.json")))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        tracks.push(t);
    }
    let mut workflows: Vec<(PathBuf, WorkflowFile)> = std::fs::read_dir(&wfs)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .filter_map(|p| {
                    std::fs::read(&p)
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .map(|w| (p, w))
                })
                .collect()
        })
        .unwrap_or_default();
    workflows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((tracks, workflows.into_iter().map(|(_, w)| w).collect()))
}

fn line_time(l: &ParsedLine) -> Option<Time> {
    use crate::claude::parse::Parsed;
    match &l.parsed {
        Parsed::Assistant(a) => a.env.timestamp,
        Parsed::User(u) => u.env.timestamp,
        Parsed::System(s) => s.env.timestamp,
        _ => None,
    }
}

/// Every line of every track in time order. Lines without a timestamp keep the time of
/// the line before them in the same file, so they stay next to it.
fn timeline(tracks: &[Track]) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ti, t) in tracks.iter().enumerate() {
        let mut last = None;
        for (li, l) in t.lines.iter().enumerate() {
            if let Some(at) = line_time(l) {
                last = Some(at);
            }
            steps.push(Step {
                at: last,
                track: ti,
                line: li,
            });
        }
    }
    // A stable sort keeps file order for equal times and puts untimed lead-in first.
    steps.sort_by_key(|s| s.at);
    steps
}

async fn play(
    c: &ReplayConfig,
    session: &SessionId,
    tracks: Vec<Track>,
    workflows: Vec<WorkflowFile>,
    sink: &mpsc::Sender<Input>,
    stop: &CancellationToken,
) {
    macro_rules! send {
        ($input:expr) => {
            if stop.is_cancelled() || sink.send($input).await.is_err() {
                return;
            }
        };
    }
    let steps = timeline(&tracks);
    // Date everything from the first recorded moment, not the Unix epoch.
    if let Some(at) = steps.iter().find_map(|s| s.at) {
        send!(Input::Tick { now: at });
    }
    let mut announced = vec![false; tracks.len()];
    let mut prev: Option<Time> = None;
    let mut last_tick: Option<Time> = None;
    let mut i = 0;
    while i < steps.len() {
        // At full speed, batch a run of consecutive lines from one file.
        let mut j = i + 1;
        if c.speed == Speed::Max {
            while j < steps.len() && steps[j].track == steps[i].track && j - i < 2000 {
                j += 1;
            }
        }
        let step = &steps[i];
        let t = &tracks[step.track];
        if let (Speed::Times(f), Some(at), Some(p)) = (c.speed, step.at, prev)
            && f > 0.0
        {
            let gap = (at - p)
                .to_std()
                .unwrap_or_default()
                .div_f64(f)
                .min(c.max_gap);
            if !gap.is_zero() {
                tokio::select! {
                    _ = stop.cancelled() => return,
                    _ = tokio::time::sleep(gap) => {}
                }
            }
        }
        let first = !announced[step.track];
        if first {
            announced[step.track] = true;
            if let (AgentRef::Sub(id), Some(meta)) = (&t.agent, &t.meta) {
                send!(Input::SubagentMeta {
                    session: session.clone(),
                    agent: id.clone(),
                    meta: meta.clone(),
                    transcript_mtime: None
                });
            }
            send!(Input::Attached {
                session: session.clone(),
                agent: t.agent.clone(),
                path: t.path.clone(),
                truncated_before: t.truncated_before
            });
            if t.agent == AgentRef::Root
                && let Ok(Some((usd, offset, at))) = cost_anchor(&t.path, 128 << 20)
            {
                send!(Input::CostAnchor {
                    session: session.clone(),
                    usd,
                    offset,
                    at
                });
            }
        }
        let lines: Vec<ParsedLine> = steps[i..j]
            .iter()
            .map(|s| tracks[s.track].lines[s.line].clone())
            .collect();
        let hi = lines.last().map(|l| l.offset + l.len).unwrap_or(0);
        // Unreadable lines have no time; they go with the file's first batch.
        let errors = if first { t.errors.clone() } else { Vec::new() };
        send!(Input::Lines {
            session: session.clone(),
            agent: t.agent.clone(),
            lines,
            errors,
            end_offset: hi,
            mtime: None
        });
        let at = steps[j - 1].at;
        if let Some(now) = at
            && at != last_tick
        {
            last_tick = at;
            send!(Input::Tick { now });
        }
        prev = at.or(prev);
        i = j;
    }
    // Tracks with no lines still announce their meta.
    for (ti, t) in tracks.iter().enumerate() {
        if !announced[ti]
            && let (AgentRef::Sub(id), Some(meta)) = (&t.agent, &t.meta)
        {
            send!(Input::SubagentMeta {
                session: session.clone(),
                agent: id.clone(),
                meta: meta.clone(),
                transcript_mtime: None
            });
        }
    }
    for file in workflows {
        send!(Input::Workflow {
            session: session.clone(),
            file
        });
    }
    if let Some(p) = prev {
        send!(Input::Tick {
            now: p + chrono::Duration::seconds(1)
        });
    }
}

/// Parses `max` or a number such as `10` or `10x`.
pub fn parse_speed(s: &str) -> Result<Speed, String> {
    if s.eq_ignore_ascii_case("max") {
        return Ok(Speed::Max);
    }
    s.trim_end_matches(['x', 'X'])
        .parse::<f64>()
        .ok()
        .filter(|f| *f > 0.0)
        .map(Speed::Times)
        .ok_or_else(|| format!("speed must be a positive number or `max`, not `{s}`"))
}
