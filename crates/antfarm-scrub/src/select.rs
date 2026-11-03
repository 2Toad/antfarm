//! Picks what goes into a fixture: a tail of the main transcript, the sub-agents it
//! refers to, and the workflows those touch. Nothing here scrubs; it only reads bounded
//! windows and keeps the lines antfarm parses.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use antfarm_core::claude::parse::{Parsed, parse_line};
use antfarm_core::tail::{TailConfig, Tailer};
use anyhow::{Context, Result, bail};
use memchr::memmem;

use crate::json::Json;
use crate::scrub::replace_image_data;

/// One transcript window: the lines antfarm would parse, with image data already cut.
#[derive(Debug, Default)]
pub struct Window {
    pub lines: Vec<Json>,
    pub truncated_before: Option<u64>,
    /// Lines of a wanted type that did not parse and were left out.
    pub dropped: u64,
    pub ignored: BTreeMap<String, u64>,
    pub versions: BTreeSet<String>,
}

impl Window {
    /// The window as JSONL bytes, one line per entry with a trailing newline.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for l in &self.lines {
            out.extend(l.to_vec());
            out.push(b'\n');
        }
        out
    }
}

#[derive(Debug)]
pub struct Subagent {
    pub id: String,
    pub window: Window,
    pub meta: Option<Json>,
    pub spawn_depth: u64,
    pub parent: Option<String>,
}

#[derive(Debug)]
pub struct Workflow {
    pub id: String,
    pub json: Json,
}

#[derive(Debug)]
pub struct Selection {
    pub original_len: u64,
    pub main: Window,
    pub subagents: Vec<Subagent>,
    pub workflows: Vec<Workflow>,
}

pub struct Options<'a> {
    pub tail_bytes: u64,
    pub sub_tail_bytes: u64,
    pub agents: usize,
    pub workflows: usize,
    pub include_agents: &'a [String],
}

/// Reads the last `max_tail` bytes of a transcript to a line boundary and keeps the
/// lines whose type antfarm parses.
pub fn read_window(path: &Path, max_tail: u64) -> Result<Window> {
    let config = TailConfig {
        max_tail,
        ..TailConfig::default()
    };
    let mut tailer =
        Tailer::attach(path, config).with_context(|| format!("open {}", path.display()))?;
    let poll = tailer
        .poll()
        .with_context(|| format!("read {}", path.display()))?;
    let mut w = Window {
        truncated_before: tailer.truncated_before(),
        ..Window::default()
    };
    for raw in poll.lines {
        if raw.bytes.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        match parse_line(&raw.bytes, raw.offset) {
            Ok(Parsed::Ignored(t)) => *w.ignored.entry(t).or_default() += 1,
            Ok(_) => match Json::parse(&raw.bytes) {
                Ok(mut j) => {
                    if let Some(v) = j.get("version").and_then(Json::as_str) {
                        w.versions.insert(v.to_owned());
                    }
                    replace_image_data(&mut j);
                    w.lines.push(j);
                }
                Err(_) => w.dropped += 1,
            },
            Err(_) => w.dropped += 1,
        }
    }
    Ok(w)
}

struct Candidate {
    id: String,
    transcript: PathBuf,
    meta: Option<Json>,
    mtime: SystemTime,
    tool_use_id: Option<String>,
    parent: Option<String>,
    spawn_depth: u64,
}

impl Candidate {
    fn deep(&self) -> bool {
        self.spawn_depth >= 2 || self.parent.is_some()
    }
}

fn list_candidates(dir: &Path) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(out);
    };
    for entry in rd {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id) = name
            .strip_prefix("agent-")
            .and_then(|n| n.strip_suffix(".jsonl"))
        else {
            continue;
        };
        let meta_path = dir.join(format!("agent-{id}.meta.json"));
        let meta = match fs::read(&meta_path) {
            Ok(bytes) => Some(
                Json::parse(&bytes).with_context(|| format!("parse {}", meta_path.display()))?,
            ),
            Err(_) => None,
        };
        let field = |k: &str| {
            meta.as_ref()
                .and_then(|m| m.get(k))
                .and_then(Json::as_str)
                .map(str::to_owned)
        };
        out.push(Candidate {
            id: id.to_owned(),
            mtime: fs::metadata(&path)?.modified()?,
            tool_use_id: field("toolUseId"),
            parent: field("parentAgentId"),
            spawn_depth: meta
                .as_ref()
                .and_then(|m| m.get("spawnDepth"))
                .and_then(Json::as_u64)
                .unwrap_or(1),
            meta,
            transcript: path,
        });
    }
    Ok(out)
}

/// Orders sub-agent candidates: forced ones (with their parent chain) first, then the
/// ones the main window mentions, most recent first. Returns `(forced, pool)`.
fn order_agents(
    mut candidates: Vec<Candidate>,
    main_bytes: &[u8],
    opts: &Options,
) -> Result<(Vec<Candidate>, Vec<Candidate>)> {
    let mentioned = |needle: &str| memmem::find(main_bytes, needle.as_bytes()).is_some();
    let mut forced: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = opts.include_agents.to_vec();
    while let Some(id) = queue.pop() {
        let Some(c) = candidates.iter().find(|c| c.id == id) else {
            bail!("--include-agent {id}: no agent-{id}.jsonl in the subagents directory");
        };
        if forced.insert(id)
            && let Some(p) = &c.parent
        {
            queue.push(p.clone());
        }
    }
    let (mut first, rest): (Vec<Candidate>, Vec<Candidate>) =
        candidates.drain(..).partition(|c| forced.contains(&c.id));
    first.sort_by_key(|c| Reverse(c.mtime));
    let mut pool: Vec<Candidate> = rest
        .into_iter()
        .filter(|c| mentioned(&c.id) || c.tool_use_id.as_deref().is_some_and(mentioned))
        .collect();
    pool.sort_by_key(|c| Reverse(c.mtime));
    Ok((first, pool))
}

/// Reads a candidate's tail; `None` when nothing in it is a line antfarm parses, so a
/// transcript whose tail is all attachments does not take a slot.
fn take(c: Candidate, sub_tail_bytes: u64) -> Result<Option<Subagent>> {
    let window = read_window(&c.transcript, sub_tail_bytes)?;
    if window.lines.is_empty() {
        eprintln!(
            "skipping agent {}: no parseable lines in its last {sub_tail_bytes} bytes",
            c.id
        );
        return Ok(None);
    }
    Ok(Some(Subagent {
        id: c.id,
        window,
        meta: c.meta,
        spawn_depth: c.spawn_depth,
        parent: c.parent,
    }))
}

/// Chooses and reads sub-agents up to `cap`, swapping in one nested agent when none made
/// it on recency alone.
fn choose_agents(
    candidates: Vec<Candidate>,
    main_bytes: &[u8],
    opts: &Options,
) -> Result<Vec<Subagent>> {
    let (forced, pool) = order_agents(candidates, main_bytes, opts)?;
    let mut chosen = Vec::new();
    for c in forced {
        chosen.extend(take(c, opts.sub_tail_bytes)?);
    }
    let mut pool = pool.into_iter().peekable();
    while chosen.len() < opts.agents {
        let Some(c) = pool.next() else { break };
        chosen.extend(take(c, opts.sub_tail_bytes)?);
    }
    let deep = |s: &Subagent| s.spawn_depth >= 2 || s.parent.is_some();
    if opts.agents > 0 && !chosen.iter().any(deep) {
        for c in pool.filter(Candidate::deep) {
            if let Some(s) = take(c, opts.sub_tail_bytes)? {
                if chosen.len() >= opts.agents {
                    chosen.pop();
                }
                chosen.push(s);
                break;
            }
        }
    }
    Ok(chosen)
}

fn choose_workflows(
    dir: &Path,
    kept_agents: &BTreeSet<String>,
    kept_bytes: &[u8],
    cap: usize,
) -> Result<Vec<Workflow>> {
    let mut out: Vec<(SystemTime, Workflow)> = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    for entry in rd {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id) = name
            .strip_prefix("wf_")
            .and_then(|n| n.strip_suffix(".json"))
        else {
            continue;
        };
        let json =
            Json::parse(&fs::read(&path)?).with_context(|| format!("parse {}", path.display()))?;
        let mentioned = |k: &str| {
            json.get(k)
                .and_then(Json::as_str)
                .is_some_and(|s| memmem::find(kept_bytes, s.as_bytes()).is_some())
        };
        let touches_agent = json
            .get("workflowProgress")
            .and_then(Json::as_array)
            .is_some_and(|p| {
                p.iter().any(|e| {
                    e.get("agentId")
                        .and_then(Json::as_str)
                        .is_some_and(|a| kept_agents.contains(a))
                })
            });
        if touches_agent || mentioned("runId") || mentioned("taskId") {
            out.push((
                fs::metadata(&path)?.modified()?,
                Workflow {
                    id: format!("wf_{id}"),
                    json,
                },
            ));
        }
    }
    out.sort_by_key(|(mtime, _)| Reverse(*mtime));
    Ok(out.into_iter().take(cap).map(|(_, w)| w).collect())
}

pub fn select(transcript: &Path, opts: &Options) -> Result<Selection> {
    let original_len = fs::metadata(transcript)
        .with_context(|| format!("stat {}", transcript.display()))?
        .len();
    let main = read_window(transcript, opts.tail_bytes)?;
    if main.lines.is_empty() {
        bail!("no parseable lines in the last {} bytes", opts.tail_bytes);
    }
    let main_bytes = main.bytes();

    let stem = transcript
        .file_stem()
        .and_then(|s| s.to_str())
        .context("transcript has no file stem")?;
    let session_dir = transcript.parent().unwrap_or(Path::new(".")).join(stem);

    let candidates = list_candidates(&session_dir.join("subagents"))?;
    let mut subagents = choose_agents(candidates, &main_bytes, opts)?;
    subagents.sort_by(|a, b| a.id.cmp(&b.id));
    let mut kept_bytes = main_bytes;
    for s in &subagents {
        kept_bytes.extend(s.window.bytes());
    }
    let kept_ids: BTreeSet<String> = subagents.iter().map(|s| s.id.clone()).collect();
    let workflows = choose_workflows(
        &session_dir.join("workflows"),
        &kept_ids,
        &kept_bytes,
        opts.workflows,
    )?;
    Ok(Selection {
        original_len,
        main,
        subagents,
        workflows,
    })
}
