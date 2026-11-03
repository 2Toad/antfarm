//! Renders every screen's view event from the scrubbed real fixtures. The JSON goes to
//! app/fixtures/, where the UI's mock mode and component tests read it. Run with
//! `ANTFARM_WRITE_FIXTURES=1` to rewrite them; otherwise the test fails if they drift.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use antfarm_app::screens::{self, Route};
use antfarm_core::claude::cost::cost_anchor;
use antfarm_core::claude::files::RegistryEntry;
use antfarm_core::claude::parse::parse_batch;
use antfarm_core::input::{AgentRef, Input};
use antfarm_core::tail::{TailConfig, Tailer};
use antfarm_core::{AgentId, Engine, ParseStats, SessionId, Time};
use antfarm_view::Prefs;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn feed(engine: &mut Engine, session: &SessionId, path: &Path, agent: AgentRef) {
    let mut tailer = Tailer::attach(
        path,
        TailConfig {
            max_tail: u64::MAX,
            ..TailConfig::default()
        },
    )
    .unwrap();
    engine.apply(Input::Attached {
        session: session.clone(),
        agent: agent.clone(),
        path: path.into(),
        truncated_before: None,
    });
    if agent == AgentRef::Root
        && let Some((usd, offset, at)) = cost_anchor(path, u64::MAX).unwrap()
    {
        engine.apply(Input::CostAnchor {
            session: session.clone(),
            usd,
            offset,
            at,
        });
    }
    let poll = tailer.poll().unwrap();
    let (lines, errors) = parse_batch(poll.lines, &mut ParseStats::default());
    engine.apply(Input::Lines {
        session: session.clone(),
        agent,
        lines,
        errors,
        end_offset: poll.end_offset,
        mtime: None,
    });
}

fn load(e: &mut Engine, name: &str) {
    let dir = root().join("fixtures/real").join(name);
    let id = SessionId::from(name);
    feed(e, &id, &dir.join("main.jsonl"), AgentRef::Root);
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir.join("subagents"))
        .map(|rd| rd.flatten().map(|x| x.path()).collect())
        .unwrap_or_default();
    subs.sort();
    for p in subs
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
    {
        let stem = p.file_stem().unwrap().to_str().unwrap();
        let agent = AgentId::from(stem.trim_start_matches("agent-"));
        if let Ok(b) = std::fs::read(p.with_extension("meta.json")) {
            e.apply(Input::SubagentMeta {
                session: id.clone(),
                agent: agent.clone(),
                meta: serde_json::from_slice(&b).unwrap(),
                transcript_mtime: None,
            });
        }
        feed(e, &id, p, AgentRef::Sub(agent));
    }
    for p in std::fs::read_dir(dir.join("workflows"))
        .map(|rd| rd.flatten().map(|x| x.path()).collect::<Vec<_>>())
        .unwrap_or_default()
    {
        if let Ok(b) = std::fs::read(&p) {
            e.apply(Input::Workflow {
                session: id.clone(),
                file: serde_json::from_slice(&b).unwrap(),
            });
        }
    }
}

fn register(e: &mut Engine, id: &str, pid: u32, status: &str, now: Time) {
    let entry: RegistryEntry = serde_json::from_value(serde_json::json!({
        "pid": pid, "sessionId": id, "cwd": format!("/w/{id}"), "name": id, "entrypoint": "claude-vscode",
        "status": status, "waitingFor": "permission prompt", "statusUpdatedAt": (now - chrono::Duration::seconds(95)).timestamp_millis()
    }))
    .unwrap();
    e.apply(Input::Registry { entry, alive: true });
}

#[test]
fn mock_fixtures_match() {
    let mut e = Engine::new();
    load(&mut e, "live-tail");
    load(&mut e, "small-session");
    let now = e.machine().sessions["live-tail"].last_write.unwrap() + chrono::Duration::seconds(60);
    register(&mut e, "live-tail", 4242, "waiting", now);
    register(&mut e, "small-session", 4343, "busy", now);
    e.apply(Input::Tick { now });
    let snap = e.snapshot();
    let s = &snap.machine.sessions["live-tail"];
    let busiest = antfarm_view::tree_order(s)
        .into_iter()
        .filter(|a| a.parent.is_some())
        .max_by_key(|a| a.calls)
        .unwrap();
    let call = antfarm_view::waterfall(busiest)
        .last()
        .unwrap()
        .call
        .clone();
    let project_key = antfarm_view::project::repo_key(s).display().to_string();
    let lanes_end = now;
    let routes: Vec<(&str, Route)> = vec![
        ("machine", Route::Machine),
        ("project", Route::Project { key: project_key }),
        ("session", Route::Session { id: s.id.clone() }),
        (
            "session-working",
            Route::Session {
                id: SessionId::from("small-session"),
            },
        ),
        (
            "lanes",
            Route::Lanes {
                id: s.id.clone(),
                start: None,
                end: None,
                playhead: None,
                span: None,
                width: Some(1200),
            },
        ),
        (
            "lanes-rewound",
            Route::Lanes {
                id: s.id.clone(),
                start: Some(lanes_end - chrono::Duration::minutes(15)),
                end: Some(lanes_end),
                playhead: Some(lanes_end - chrono::Duration::minutes(6)),
                span: None,
                width: Some(1200),
            },
        ),
        (
            "agent",
            Route::Agent {
                session: s.id.clone(),
                agent: busiest.id.clone(),
            },
        ),
        (
            "call",
            Route::Call {
                session: s.id.clone(),
                agent: busiest.id.clone(),
                call,
            },
        ),
    ];
    // A variant where both sessions edited the same file in the same tree a few minutes
    // apart, so the project screen shows a conflict.
    let mut conflict = snap.clone();
    let shared = format!(
        "{}/src/auth/session.ts",
        conflict.machine.sessions["live-tail"].project.display()
    );
    for (id, mins, added, removed) in [("live-tail", 7, 112, 87), ("small-session", 3, 9, 2)] {
        let s = conflict.machine.sessions.get_mut(id).unwrap();
        let agent = s.root.clone();
        s.files_touched.push(antfarm_core::FileTouch {
            path: shared.clone(),
            agent,
            tool: "Edit".into(),
            at: Some(now - chrono::Duration::minutes(mins)),
            added,
            removed,
        });
    }
    let conflict_key = antfarm_view::project::repo_key(&conflict.machine.sessions["live-tail"])
        .display()
        .to_string();
    let prefs = Prefs {
        tree: true,
        ..Prefs::default()
    };
    let write = std::env::var_os("ANTFARM_WRITE_FIXTURES").is_some();
    let dir = root().join("app/fixtures");
    let mut drift = Vec::new();
    let mut all: Vec<(&str, Route, &antfarm_core::Snapshot)> =
        routes.into_iter().map(|(n, r)| (n, r, &snap)).collect();
    all.push((
        "project-conflict",
        Route::Project { key: conflict_key },
        &conflict,
    ));
    for (name, route, snap) in all {
        let event = screens::ViewEvent {
            seq: snap.seq,
            now,
            header: screens::header(snap, &prefs, "machine", now),
            body: screens::body(snap, &route, &prefs, &HashMap::new(), false, now),
        };
        assert!(
            !matches!(event.body, screens::Body::Gone { .. }),
            "{name} rendered nothing"
        );
        let json = serde_json::to_string_pretty(&event).unwrap() + "\n";
        let path = dir.join(format!("{name}.json"));
        if write {
            std::fs::write(&path, &json).unwrap();
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
            drift.push(name);
        }
    }
    assert!(
        drift.is_empty(),
        "mock fixtures out of date: {drift:?}; rerun with ANTFARM_WRITE_FIXTURES=1 and review the diff"
    );
}
