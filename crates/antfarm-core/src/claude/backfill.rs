//! Reads the history before a transcript's tail boundary in the background.
//!
//! Attach reads only a bounded tail so live state shows up at once. For each transcript
//! cut that way, the source queues a job here. One thread runs the jobs in order: it
//! reads backward from the boundary in chunks with [`BackReader`], decodes each chunk
//! into compact [`CallRecord`]s and file edits, and sends them to the engine as
//! [`Input::Backfill`] batches, newest first. It never touches the live reducer's state,
//! so status, names, and context fill stay as the tail left them.
//!
//! Going backward, a call's result is read before the call itself. Results wait in a
//! map until their call turns up. A result after the boundary was already read by the
//! tail; the engine kept it as an orphan and pairs it when the batch arrives.

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;

use super::line::{Block, UserContent};
use super::parse::{Parsed, RawLine, parse_batch};
use super::reduce::{line_delta, summarize};
use super::source::SourceStats;
use crate::input::{AgentRef, ForkLink, Input};
use crate::model::{
    AgentId, CallRecord, FileTouch, LineRefs, ParseStats, ResultStatus, SessionId, Time, ToolCallId,
};
use crate::tail::{BackReader, TailConfig};

/// One transcript's history to read.
#[derive(Debug)]
pub(crate) struct Job {
    pub id: u64,
    pub session: SessionId,
    pub agent: AgentRef,
    /// The agent the calls belong to, for file edits.
    pub agent_id: AgentId,
    pub path: PathBuf,
    /// Opened at attach, so a move before the job runs doesn't lose the file.
    pub file: File,
    pub boundary: u64,
    /// Set when the transcript was truncated or replaced; the job stops at the next chunk.
    pub cancel: Arc<AtomicBool>,
}

/// Runs jobs one at a time on a thread of its own.
pub(crate) struct Runner {
    jobs: std::sync::mpsc::Sender<Job>,
}

impl Runner {
    pub fn spawn(
        sink: mpsc::Sender<Input>,
        chunk: u64,
        pause: Duration,
        stats: Arc<Mutex<SourceStats>>,
    ) -> Self {
        let (jobs, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("antfarm-backfill".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    if run(&job, chunk, pause, &sink, &stats).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn backfill thread");
        Self { jobs }
    }

    pub fn submit(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

/// The engine is gone.
struct Closed;

fn run(
    job: &Job,
    chunk: u64,
    pause: Duration,
    sink: &mpsc::Sender<Input>,
    stats: &Mutex<SourceStats>,
) -> Result<(), Closed> {
    let send = |(calls, files, forks), read, done| {
        sink.blocking_send(Input::Backfill {
            session: job.session.clone(),
            agent: job.agent.clone(),
            job: job.id,
            calls,
            files,
            forks,
            read,
            total: job.boundary,
            done,
        })
        .map_err(|_| Closed)
    };
    let fail = |e: std::io::Error| {
        let mut s = stats.lock().unwrap();
        if s.errors.len() < 100 {
            s.errors
                .push(format!("backfill {}: {e}", job.path.display()));
        }
    };
    let file = match job.file.try_clone() {
        Ok(f) => f,
        Err(e) => {
            fail(e);
            return send(Decoded::default(), job.boundary, true);
        }
    };
    let mut reader = BackReader::new(file, job.boundary, &TailConfig::default(), chunk);
    let mut decoder = Decoder::new(job.agent_id.clone());
    let mut reported = 0;
    loop {
        if job.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let lines = match reader.next_chunk() {
            Ok(Some(lines)) => lines,
            Ok(None) => return Ok(()),
            Err(e) => {
                // The engine drops this job's batches once it hears of the truncation;
                // finishing here keeps a read error from showing as loading forever.
                fail(e);
                return send(Decoded::default(), job.boundary, true);
            }
        };
        let decoded = decoder.decode(lines);
        let read = job.boundary - reader.remaining();
        send(decoded, read, reader.remaining() == 0)?;
        let mut s = stats.lock().unwrap();
        s.bytes_read += read - reported;
        reported = read;
        s.oversized = s.oversized.max(reader.oversized());
        drop(s);
        if !pause.is_zero() {
            std::thread::sleep(pause);
        }
    }
}

/// A result read before its call was.
#[derive(Debug, Clone)]
struct Seen {
    at: Option<Time>,
    status: ResultStatus,
    offset: u64,
    /// The agent a forked skill's result names.
    fork: Option<String>,
}

/// One chunk's calls in file order, its edits one entry per path, and the forked skills
/// that returned in it.
pub(crate) type Decoded = (Vec<CallRecord>, Vec<FileTouch>, Vec<ForkLink>);

/// Turns chunks of lines, newest chunk first, into compact calls and file edits.
pub(crate) struct Decoder {
    agent: AgentId,
    results: HashMap<String, Seen>,
    names: HashMap<String, Arc<str>>,
}

impl Decoder {
    pub fn new(agent: AgentId) -> Self {
        Self {
            agent,
            results: HashMap::new(),
            names: HashMap::new(),
        }
    }

    fn name(&mut self, n: &str) -> Arc<str> {
        if let Some(a) = self.names.get(n) {
            return a.clone();
        }
        let a: Arc<str> = n.into();
        self.names.insert(n.to_owned(), a.clone());
        a
    }

    /// Decodes one chunk, which is older than every chunk before it. Returns its calls in
    /// file order and its edits, one entry per path.
    pub fn decode(&mut self, lines: Vec<RawLine>) -> Decoded {
        let mut stats = ParseStats::default();
        let (parsed, _errors) = parse_batch(lines, &mut stats);
        let mut calls = Vec::new();
        let mut files: Vec<FileTouch> = Vec::new();
        let mut forks = Vec::new();
        for line in parsed.iter().rev() {
            match &line.parsed {
                Parsed::User(u) => {
                    let UserContent::Blocks(blocks) = &u.message.content else {
                        continue;
                    };
                    let fork = u
                        .tool_use_result
                        .as_ref()
                        .filter(|r| r.status.as_deref() == Some("forked"))
                        .and_then(|r| r.agent_id.clone())
                        .filter(|a| !a.is_empty());
                    for b in blocks {
                        if let Block::ToolResult {
                            tool_use_id,
                            is_error,
                            ..
                        } = b
                        {
                            let status = if is_error.unwrap_or(false) {
                                ResultStatus::Error
                            } else {
                                ResultStatus::Ok
                            };
                            self.results.insert(
                                tool_use_id.clone(),
                                Seen {
                                    at: u.env.timestamp,
                                    status,
                                    offset: line.offset,
                                    fork: fork.clone(),
                                },
                            );
                        }
                    }
                }
                Parsed::Assistant(a) => {
                    let at = a.env.timestamp;
                    for b in a.message.content.iter().rev() {
                        let Block::ToolUse { id, name, input } = b else {
                            continue;
                        };
                        let seen = self.results.remove(id);
                        self.touch(&mut files, name, input, at);
                        if let Some(agent) = seen.as_ref().and_then(|s| s.fork.as_deref())
                            && name == super::subagents::FORK_TOOL
                        {
                            forks.push(ForkLink {
                                call: ToolCallId::from(id.as_str()),
                                agent: AgentId::from(agent),
                                at: seen.as_ref().and_then(|s| s.at),
                            });
                        }
                        calls.push(CallRecord {
                            id: ToolCallId::from(id.as_str()),
                            name: self.name(name),
                            summary: summarize(name, input).into(),
                            at,
                            finished: seen.as_ref().and_then(|s| s.at),
                            status: seen.as_ref().map_or(ResultStatus::Pending, |s| s.status),
                            lines: LineRefs {
                                call: Some(line.offset),
                                result: seen.as_ref().map(|s| s.offset),
                            },
                            background: None,
                        });
                    }
                }
                _ => {}
            }
        }
        calls.reverse();
        forks.reverse();
        (calls, files, forks)
    }

    /// Adds an edit to the chunk's files. Lines are visited newest first, so the first
    /// sight of a path sets its time and tool.
    fn touch(&self, files: &mut Vec<FileTouch>, name: &str, input: &Value, at: Option<Time>) {
        if !matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
            return;
        }
        let Some(path) = input
            .get("file_path")
            .or_else(|| input.get("notebook_path"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let (added, removed) = line_delta(name, input);
        match files.iter_mut().find(|f| f.path == path) {
            Some(f) => {
                f.added += added;
                f.removed += removed;
            }
            None => files.push(FileTouch {
                path: path.to_owned(),
                agent: self.agent.clone(),
                tool: name.to_owned(),
                at,
                added,
                removed,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(offset: u64, s: &str) -> RawLine {
        RawLine {
            offset,
            bytes: s.as_bytes().to_vec(),
        }
    }

    const CALL_A: &str = r#"{"type":"assistant","timestamp":"2026-11-02T10:00:00Z","message":{"id":"m1","content":[{"type":"tool_use","id":"ta","name":"Edit","input":{"file_path":"/p/a.rs","old_string":"x","new_string":"y\nz"}}]}}"#;
    const RESULT_A: &str = r#"{"type":"user","timestamp":"2026-11-02T10:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"ta","content":"ok"}]}}"#;
    const CALL_B: &str = r#"{"type":"assistant","timestamp":"2026-11-02T10:00:05Z","message":{"id":"m2","content":[{"type":"tool_use","id":"tb","name":"Bash","input":{"command":"ls"}}]}}"#;
    const RESULT_B: &str = r#"{"type":"user","timestamp":"2026-11-02T10:00:06Z","message":{"content":[{"type":"tool_result","tool_use_id":"tb","is_error":true,"content":"no"}]}}"#;

    #[test]
    fn pairs_results_read_before_their_calls_across_chunks() {
        let mut d = Decoder::new(AgentId::from("root"));
        // Newest chunk first: B's result, then a chunk with A, A's result, and B's call.
        let (calls, _, _) = d.decode(vec![raw(300, RESULT_B)]);
        assert!(calls.is_empty());
        let (calls, files, _) =
            d.decode(vec![raw(0, CALL_A), raw(100, RESULT_A), raw(200, CALL_B)]);
        let ids: Vec<_> = calls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["ta", "tb"]);
        assert_eq!(calls[0].status, ResultStatus::Ok);
        assert_eq!(
            calls[0].lines,
            LineRefs {
                call: Some(0),
                result: Some(100)
            }
        );
        assert_eq!(calls[1].status, ResultStatus::Error);
        assert_eq!(calls[1].lines.result, Some(300));
        assert_eq!(&*calls[1].summary, "ls");
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].added, files[0].removed), (2, 1));
    }

    #[test]
    fn a_forked_skill_result_becomes_a_fork_link() {
        let mut d = Decoder::new(AgentId::from("root"));
        let call = r#"{"type":"assistant","timestamp":"2026-11-02T10:00:00Z","message":{"id":"m1","content":[{"type":"tool_use","id":"tsk","name":"Skill","input":{"skill":"code-review"}}]}}"#;
        let result = r#"{"type":"user","timestamp":"2026-11-02T10:05:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"tsk","content":"done"}]},"toolUseResult":{"agentId":"afk","commandName":"code-review","status":"forked","success":true}}"#;
        let (calls, _, forks) = d.decode(vec![raw(0, call), raw(200, result)]);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            forks,
            [ForkLink {
                call: ToolCallId::from("tsk"),
                agent: AgentId::from("afk"),
                at: Some("2026-11-02T10:05:00Z".parse().unwrap()),
            }]
        );
    }

    #[test]
    fn a_call_with_no_result_before_the_boundary_stays_pending() {
        let mut d = Decoder::new(AgentId::from("root"));
        let (calls, _, _) = d.decode(vec![raw(0, CALL_B)]);
        assert_eq!(calls[0].status, ResultStatus::Pending);
        assert_eq!(calls[0].finished, None);
    }
}
