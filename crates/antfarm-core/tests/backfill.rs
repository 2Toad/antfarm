//! Backfill against a scrubbed real session: attach with a tail far smaller than the
//! transcripts, let backfill read the rest, and compare with reading every file whole.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use antfarm_core::claude::{ClaudeCodeSource, SourceConfig};
use antfarm_core::proc::FakeProbe;
use antfarm_core::source::{Clock, Handle, Hub};
use antfarm_core::{Event, HistoryState, Paths, Session, Snapshot};
use tokio_stream::StreamExt;

const SID: &str = "31c1b9f3-721f-4086-a7fa-b2b5d2ab151b";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real/small-session")
}

/// A Claude home holding the fixture as a live session under `slug`.
fn home(slug: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let sessions = home.path().join("sessions");
    let project = home.path().join("projects").join(slug);
    let subs = project.join(SID).join("subagents");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::create_dir_all(&subs).unwrap();
    std::fs::copy(
        fixture().join("main.jsonl"),
        project.join(format!("{SID}.jsonl")),
    )
    .unwrap();
    for e in std::fs::read_dir(fixture().join("subagents")).unwrap() {
        let p = e.unwrap().path();
        std::fs::copy(&p, subs.join(p.file_name().unwrap())).unwrap();
    }
    std::fs::write(
        sessions.join("4242.json"),
        format!(r#"{{"pid":4242,"sessionId":"{SID}","cwd":"/w/app","status":"idle"}}"#),
    )
    .unwrap();
    home
}

fn config(home: &Path, tail: u64, backfill: bool, chunk: u64) -> SourceConfig {
    let mut c = SourceConfig::new(Paths {
        claude_home: home.to_path_buf(),
    });
    c.poll_only = true;
    c.poll_interval = Duration::from_millis(50);
    c.main_tail = tail;
    c.sub_tail = tail;
    c.sub_recent = Duration::from_secs(10 * 365 * 86_400);
    c.backfill = backfill;
    c.backfill_chunk = chunk;
    c
}

/// A config whose backfill takes long enough for a test to act while it runs: about 40
/// chunks of the fixture, each followed by a short pause, so about a second in all.
fn slow(home: &Path) -> SourceConfig {
    let mut c = config(home, 32 << 10, true, 64 << 10);
    c.backfill_pause = Duration::from_millis(30);
    c
}

fn start(c: SourceConfig) -> Handle {
    let probe = FakeProbe {
        alive: vec![4242],
        ancestry: Vec::new(),
    };
    Hub::spawn(
        Box::new(ClaudeCodeSource::with_probe(c, Box::new(probe))),
        Clock::Wall,
    )
}

/// Distinct `tool_use` ids in the fixture's transcripts.
fn tool_uses() -> usize {
    let mut ids = BTreeSet::new();
    let mut files = vec![fixture().join("main.jsonl")];
    for e in std::fs::read_dir(fixture().join("subagents")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "jsonl") {
            files.push(p);
        }
    }
    for f in files {
        for line in std::fs::read_to_string(f).unwrap().lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            for b in v["message"]["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_use" {
                    ids.insert(b["id"].as_str().unwrap().to_owned());
                }
            }
        }
    }
    ids.len()
}

fn calls(s: &Session) -> u64 {
    s.agents.values().map(|a| a.calls).sum()
}

async fn wait_for(h: &Handle, what: &str, f: impl Fn(&Snapshot) -> bool) -> Arc<Snapshot> {
    let mut rx = h.snapshots();
    // Generous, for a shared CI runner; a passing run takes a few seconds.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let snap = rx.borrow_and_update().clone();
        if f(&snap) {
            return snap;
        }
        if tokio::time::timeout_at(deadline, rx.changed())
            .await
            .is_err()
        {
            let s = snap.machine.sessions.get(SID);
            panic!(
                "timed out waiting for: {what}; calls {:?}, history {:?}",
                s.map(calls),
                s.map(|s| s.history)
            );
        }
    }
}

/// Everything the swimlanes, call lists, and totals read, per agent.
#[derive(Debug, PartialEq)]
struct Picture {
    calls: BTreeMap<String, u64>,
    lanes: BTreeMap<String, Vec<String>>,
    files: BTreeMap<String, (String, String, u64, u64)>,
}

fn picture(s: &Session) -> Picture {
    Picture {
        calls: s
            .agents
            .values()
            .map(|a| (a.id.as_str().to_owned(), a.calls))
            .collect(),
        lanes: s
            .agents
            .values()
            .map(|a| {
                let calls = a
                    .all_calls()
                    .map(|c| {
                        format!(
                            "{} {} {:?} {:?} {:?} {}",
                            c.call.as_str(),
                            c.name,
                            c.at,
                            c.finished,
                            c.status,
                            c.summary
                        )
                    })
                    .collect();
                (a.id.as_str().to_owned(), calls)
            })
            .collect(),
        files: s
            .files_touched
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    (
                        f.agent.as_str().to_owned(),
                        f.tool.clone(),
                        f.added,
                        f.removed,
                    ),
                )
            })
            .collect(),
    }
}

async fn full_picture() -> Picture {
    let home = home("-w-app");
    let h = start(config(home.path(), 1 << 30, false, 1 << 20));
    let want = tool_uses() as u64;
    let snap = wait_for(&h, "every call read whole", |s| {
        s.machine
            .sessions
            .get(SID)
            .is_some_and(|x| calls(x) == want)
    })
    .await;
    let s = &snap.machine.sessions[SID];
    assert_eq!(s.history, HistoryState::Complete);
    assert_eq!(s.truncated_before, None);
    let p = picture(s);
    h.stop();
    p
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_matches_reading_every_transcript_whole() {
    let full = full_picture().await;
    let home = home("-w-app");
    let h = start(config(home.path(), 32 << 10, true, 8 << 10));
    let mut events = Box::pin(h.event_stream());
    let snap = wait_for(&h, "backfill complete", |s| {
        s.machine.sessions.get(SID).is_some_and(|x| {
            x.history == HistoryState::Complete
                && x.agents.len() > 1
                && calls(x) as usize == tool_uses()
        })
    })
    .await;
    let s = &snap.machine.sessions[SID];
    assert_eq!(s.truncated_before, None, "the marker is gone");
    let back = picture(s);
    assert_eq!(back.calls, full.calls);
    for (agent, lanes) in &full.lanes {
        assert_eq!(&back.lanes[agent], lanes, "calls of {agent}");
    }
    assert_eq!(back.files, full.files);
    assert!(!back.files.is_empty());
    // Sub-agents were cut at the same small tail and backfilled too.
    assert!(full.calls.len() > 1);
    let backfilled_subs = s
        .agents
        .values()
        .filter(|a| !a.is_root() && !a.older.is_empty())
        .count();
    assert!(backfilled_subs > 0, "some sub-agent history was backfilled");

    // A backfilled call's input and result read back from the transcript by offset.
    let root = s.root_agent();
    let path = root.transcript.as_deref().unwrap();
    let old = root
        .older
        .iter()
        .find(|c| c.status != antfarm_core::ResultStatus::Pending && &*c.name == "Bash")
        .expect("a finished backfilled Bash call");
    let input = antfarm_core::claude::detail::read_input(path, old.lines, old.id.as_str())
        .expect("input read back");
    assert!(input.get("command").is_some());
    let result = antfarm_core::claude::detail::read_result(path, old.lines, old.id.as_str())
        .expect("result read back");
    assert_eq!(
        result.is_error,
        old.status == antfarm_core::ResultStatus::Error
    );

    let saw = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = events.next().await {
            if matches!(e.event, Event::HistoryComplete) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(saw, Ok(true), "HistoryComplete was emitted");
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_shows_while_backfill_reads() {
    let home = home("-w-app");
    let h = start(slow(home.path()));
    let snap = wait_for(&h, "loading with progress", |s| {
        s.machine
            .sessions
            .get(SID)
            .is_some_and(|x| matches!(x.history, HistoryState::Loading { read, .. } if read > 0))
    })
    .await;
    let HistoryState::Loading { read, total } = snap.machine.sessions[SID].history else {
        unreachable!()
    };
    assert!(read < total);
    wait_for(&h, "backfill complete", |s| {
        s.machine.sessions[SID].history == HistoryState::Complete
    })
    .await;
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_flag_off_leaves_history_partial() {
    let home = home("-w-app");
    let h = start(config(home.path(), 32 << 10, false, 8 << 10));
    // Every tail is read once the count stops changing; a slow machine reads them over
    // several passes, so "some calls" alone can land before the last one.
    let mut tail = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let n = h
            .snapshots()
            .borrow()
            .machine
            .sessions
            .get(SID)
            .map_or(0, calls);
        if n > 0 && n == tail {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "tails never settled at {n}"
        );
        tail = n;
    }
    // With backfill off, nothing is added after the tails.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let snap = h.snapshots().borrow().clone();
    let s = &snap.machine.sessions[SID];
    assert_eq!(s.history, HistoryState::Partial);
    assert!(s.truncated_before.is_some());
    assert_eq!(calls(s), tail);
    assert!((calls(s) as usize) < tool_uses());
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transcript_replaced_during_backfill_has_no_duplicate_calls() {
    let home = home("-w-app");
    let main = home.path().join(format!("projects/-w-app/{SID}.jsonl"));
    let h = start(slow(home.path()));
    wait_for(&h, "loading", |s| {
        s.machine
            .sessions
            .get(SID)
            .is_some_and(|x| matches!(x.history, HistoryState::Loading { read, .. } if read > 0))
    })
    .await;
    // Rewrite the main transcript as its first 40 lines, the way a rewrite replaces it.
    let text = std::fs::read_to_string(fixture().join("main.jsonl")).unwrap();
    let head: String = text.lines().take(40).map(|l| format!("{l}\n")).collect();
    let tmp = main.with_extension("tmp");
    std::fs::write(&tmp, &head).unwrap();
    std::fs::rename(&tmp, &main).unwrap();
    let head_calls: BTreeSet<String> = head
        .lines()
        .flat_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "tool_use")
                .map(|b| b["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let snap = wait_for(&h, "root re-read and every backfill done", |s| {
        s.machine.sessions.get(SID).is_some_and(|x| {
            let ids: BTreeSet<String> = x
                .root_agent()
                .all_calls()
                .map(|c| c.call.as_str().to_owned())
                .collect();
            x.history == HistoryState::Complete && head_calls.is_subset(&ids)
        })
    })
    .await;
    // Give a cancelled job's last batch time to arrive; the engine must drop it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = snap;
    let snap = h.snapshots().borrow().clone();
    let root = snap.machine.sessions[SID].root_agent();
    let ids: Vec<&str> = root.all_calls().map(|c| c.call.as_str()).collect();
    let unique: BTreeSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "no duplicate calls");
    assert_eq!(
        root.calls as usize,
        ids.len(),
        "the call count matches the calls held"
    );
    // Nothing from before the old tail boundary came back once the file was replaced.
    assert!(root.older.is_empty());
    h.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transcript_that_moves_during_backfill_is_followed() {
    let full = full_picture().await;
    let home = home("-w-app");
    let h = start(slow(home.path()));
    wait_for(&h, "loading", |s| {
        s.machine
            .sessions
            .get(SID)
            .is_some_and(|x| matches!(x.history, HistoryState::Loading { read, .. } if read > 0))
    })
    .await;
    // Entering a worktree moves the transcript and its folder to another project folder.
    let from = home.path().join("projects/-w-app");
    let to = home.path().join("projects/-w-app-wt");
    std::fs::rename(&from, &to).unwrap();
    let want = tool_uses() as u64;
    let snap = wait_for(&h, "backfill complete after the move", |s| {
        s.machine
            .sessions
            .get(SID)
            .is_some_and(|x| x.history == HistoryState::Complete && calls(x) == want)
    })
    .await;
    let s = &snap.machine.sessions[SID];
    assert_eq!(picture(s).calls, full.calls);
    assert_eq!(
        s.root_agent().transcript.as_deref(),
        Some(to.join(format!("{SID}.jsonl")).as_path())
    );
    h.stop();
}
