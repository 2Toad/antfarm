//! Drives `ClaudeCodeSource` through a hub against a temporary Claude home, appending
//! lines and rewriting the registry the way Claude Code does, and checks the snapshots.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use antfarm_core::claude::{ClaudeCodeSource, SourceConfig};
use antfarm_core::proc::FakeProbe;
use antfarm_core::source::{Clock, Handle, Hub};
use antfarm_core::{Event, Paths, SessionStatus, Snapshot, WaitingKind};
use tokio_stream::StreamExt;

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn append(path: &Path, line: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(f, "{line}").unwrap();
}

fn registry(dir: &Path, status: &str) {
    let body = format!(
        r#"{{"pid":4242,"sessionId":"s1","cwd":"/w/app","name":"app","startedAt":1793657740567,"status":"{status}","statusUpdatedAt":{}}}"#,
        chrono::Utc::now().timestamp_millis()
    );
    // Claude Code rewrites the file in place; a rename keeps the read atomic here.
    let tmp = dir.join("4242.json.tmp");
    std::fs::write(&tmp, body).unwrap();
    std::fs::rename(tmp, dir.join("4242.json")).unwrap();
}

fn tool_use(id: &str, name: &str) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"{}","cwd":"/w/app","sessionId":"s1","message":{{"id":"m-{id}","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{{"command":"cargo test","description":"spawned helper"}}}}],"usage":{{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1000,"cache_creation_input_tokens":0}}}}}}"#,
        now()
    )
}

fn tool_result(id: &str, extra: &str) -> String {
    format!(
        r#"{{"type":"user","timestamp":"{}","sessionId":"s1","message":{{"content":[{{"type":"tool_result","tool_use_id":"{id}","content":"ok"}}]}}{extra}}}"#,
        now()
    )
}

async fn wait_for(h: &Handle, what: &str, f: impl Fn(&Snapshot) -> bool) -> Arc<Snapshot> {
    let mut rx = h.snapshots();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snap = rx.borrow_and_update().clone();
        if f(&snap) {
            return snap;
        }
        if tokio::time::timeout_at(deadline, rx.changed())
            .await
            .is_err()
        {
            panic!(
                "timed out waiting for: {what}\n{:#?}",
                snap.machine
                    .sessions
                    .values()
                    .map(|s| (&s.status, s.agents.len()))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn follows_a_session_through_its_life() {
    let home = tempfile::tempdir().unwrap();
    let sessions = home.path().join("sessions");
    let project = home.path().join("projects/-w-app");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("s1.jsonl");
    append(&transcript, &tool_use("t1", "Bash"));
    registry(&sessions, "busy");

    let mut config = SourceConfig::new(Paths {
        claude_home: home.path().to_path_buf(),
    });
    config.poll_only = true;
    config.poll_interval = Duration::from_millis(100);
    let source = ClaudeCodeSource::with_probe(config, Box::new(FakeProbe { alive: vec![4242] }));
    let h = Hub::spawn(Box::new(source), Clock::Wall);
    let mut events = Box::pin(h.event_stream());

    let s = wait_for(&h, "session working on t1", |s| {
        s.machine
            .sessions
            .get("s1")
            .is_some_and(|x| x.status == SessionStatus::Working && x.open_calls().count() == 1)
    })
    .await;
    assert_eq!(s.machine.sessions["s1"].name.as_deref(), Some("app"));

    append(&transcript, &tool_result("t1", ""));
    wait_for(&h, "t1 closed", |s| {
        s.machine.sessions["s1"].open_calls().count() == 0
    })
    .await;

    // A background sub-agent: spawn, async result, then its meta and transcript appear.
    append(&transcript, &tool_use("t2", "Agent"));
    append(
        &transcript,
        &tool_result(
            "t2",
            r#","toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a1"}"#,
        ),
    );
    let subs = project.join("s1/subagents");
    std::fs::create_dir_all(&subs).unwrap();
    std::fs::write(
        subs.join("agent-a1.meta.json"),
        r#"{"agentType":"Explore","toolUseId":"t2","spawnDepth":1,"requestShape":"background"}"#,
    )
    .unwrap();
    append(
        &subs.join("agent-a1.jsonl"),
        &tool_use("t3", "Grep").replace("\"sessionId\"", "\"isSidechain\":true,\"sessionId\""),
    );
    wait_for(&h, "sub-agent a1 linked and working", |s| {
        s.machine.sessions["s1"].agents.get("a1").is_some_and(|a| {
            a.kind == "Explore"
                && a.open.len() == 1
                && a.spawned_by.as_ref().is_some_and(|c| c.as_str() == "t2")
        })
    })
    .await;

    // The root asks to run something and the registry goes idle: waiting for permission.
    append(&transcript, &tool_use("t4", "Bash"));
    registry(&sessions, "idle");
    wait_for(&h, "waiting for permission", |s| {
        s.machine.sessions["s1"].status == SessionStatus::Waiting(WaitingKind::Permission)
    })
    .await;

    std::fs::remove_file(sessions.join("4242.json")).unwrap();
    wait_for(&h, "exited", |s| {
        s.machine.sessions["s1"].status == SessionStatus::Exited
    })
    .await;

    h.stop();
    let mut seen = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_millis(200), events.next()).await {
        seen.push(e.event.clone());
    }
    let pos = |f: &dyn Fn(&Event) -> bool| {
        seen.iter()
            .position(f)
            .unwrap_or_else(|| panic!("missing event in {seen:#?}"))
    };
    let started =
        pos(&|e| matches!(e, Event::ToolCallStarted { call, .. } if call.as_str() == "t1"));
    let spawned =
        pos(&|e| matches!(e, Event::AgentSpawned { agent, .. } if agent.as_str() == "a1"));
    let exited = pos(&|e| matches!(e, Event::SessionExited));
    assert!(started < spawned && spawned < exited);
    h.join().await.unwrap();
}

/// With a one-minute poll, a change can only show up within seconds if the native
/// watcher reported it.
#[tokio::test(flavor = "multi_thread")]
async fn native_watcher_wakes_the_source() {
    let home = tempfile::tempdir().unwrap();
    let sessions = home.path().join("sessions");
    let project = home.path().join("projects/-w-app");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("s1.jsonl");
    append(&transcript, &tool_use("t1", "Bash"));
    registry(&sessions, "busy");

    let mut config = SourceConfig::new(Paths {
        claude_home: home.path().to_path_buf(),
    });
    config.poll_interval = Duration::from_secs(60);
    let source = ClaudeCodeSource::with_probe(config, Box::new(FakeProbe { alive: vec![4242] }));
    let h = Hub::spawn(Box::new(source), Clock::Wall);
    wait_for(&h, "first pass", |s| {
        s.machine
            .sessions
            .get("s1")
            .is_some_and(|x| x.open_calls().count() == 1)
    })
    .await;
    // Give FSEvents a moment to arm on the new directories before writing.
    tokio::time::sleep(Duration::from_millis(500)).await;
    append(&transcript, &tool_result("t1", ""));
    wait_for(&h, "result seen through the watcher", |s| {
        s.machine.sessions["s1"].open_calls().count() == 0
    })
    .await;
    h.stop();
    h.join().await.unwrap();
}
