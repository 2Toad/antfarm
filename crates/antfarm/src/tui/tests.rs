//! Golden screens: each real fixture is fed through the engine, then every screen is
//! rendered into a test buffer at 120 by 40 and 80 by 24 and compared as text.

use std::path::{Path, PathBuf};

use antfarm_core::claude::cost::cost_anchor;
use antfarm_core::claude::files::RegistryEntry;
use antfarm_core::claude::parse::parse_batch;
use antfarm_core::input::{AgentRef, Input};
use antfarm_core::tail::{TailConfig, Tailer};
use antfarm_core::{AgentId, Engine, ParseStats, SessionId, Snapshot, Time};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::app::{App, Focus, Screen};
use super::draw;
use super::theme::Theme;
use antfarm_view::view;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/real")
        .join(name)
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
        backfill: None,
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

/// Feeds a fixture and returns the engine, the session id, and a `now` one minute after
/// its last line.
fn load(name: &str) -> (Engine, SessionId, Time) {
    let dir = fixture(name);
    let id = SessionId::from(name);
    let mut e = Engine::new();
    feed(&mut e, &id, &dir.join("main.jsonl"), AgentRef::Root);
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
        feed(&mut e, &id, p, AgentRef::Sub(agent));
    }
    let now = e.machine().sessions[&id].last_write.unwrap() + chrono::Duration::seconds(60);
    e.apply(Input::Tick { now });
    (e, id, now)
}

/// Puts the session in the registry as a live process with `status`.
fn register(e: &mut Engine, id: &SessionId, status: &str, now: Time) {
    let entry: RegistryEntry = serde_json::from_value(serde_json::json!({
        "pid": 4242, "sessionId": id.as_str(), "cwd": "/w/app", "name": id.as_str(), "entrypoint": "claude-vscode",
        "status": status, "waitingFor": "permission prompt", "statusUpdatedAt": (now - chrono::Duration::seconds(95)).timestamp_millis()
    }))
    .unwrap();
    e.apply(Input::Registry { entry, alive: true });
    e.apply(Input::Tick { now });
}

fn render(app: &mut App, snap: &Snapshot, now: Time, w: u16, h: u16) -> String {
    antfarm_view::fmt::force_utc();
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| draw(f, app, snap, now, &Theme { color: false }))
        .unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        let line: String = (0..w).map(|x| buf[(x, y)].symbol().to_owned()).collect();
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn screens(name: &str, waiting: Option<&str>) {
    let (mut e, id, now) = load(name);
    if let Some(status) = waiting {
        register(&mut e, &id, status, now);
    }
    let snap = e.snapshot();
    let tag = waiting
        .map(|s| format!("{name}-{s}"))
        .unwrap_or_else(|| name.to_owned());
    for (w, h) in [(120, 40), (80, 24)] {
        let mut app = App::default();
        insta::assert_snapshot!(
            format!("{tag}-machine-{w}x{h}"),
            render(&mut app, &snap, now, w, h)
        );
        app.stack.push(Screen::Session {
            session: id.clone(),
        });
        insta::assert_snapshot!(
            format!("{tag}-session-{w}x{h}"),
            render(&mut app, &snap, now, w, h)
        );
        let s = &snap.machine.sessions[&id];
        let busiest = view::tree_order(s)
            .into_iter()
            .filter(|a| a.parent.is_some())
            .max_by_key(|a| a.calls)
            .map(|a| a.id.clone());
        if let Some(agent) = busiest {
            app.stack.push(Screen::Agent {
                session: id.clone(),
                agent: agent.clone(),
            });
            insta::assert_snapshot!(
                format!("{tag}-agent-{w}x{h}"),
                render(&mut app, &snap, now, w, h)
            );
            let call = view::waterfall(&s.agents[&agent])
                .last()
                .map(|x| x.call.clone())
                .unwrap();
            app.stack.push(Screen::Call {
                session: id.clone(),
                agent,
                call,
            });
            insta::assert_snapshot!(
                format!("{tag}-call-{w}x{h}"),
                render(&mut app, &snap, now, w, h)
            );
        }
    }
}

#[test]
fn live_tail_screens() {
    screens("live-tail", None);
}

#[test]
fn small_session_screens() {
    screens("small-session", None);
}

#[test]
fn waiting_screens() {
    screens("live-tail", Some("waiting"));
}

#[test]
fn help_and_tiny_terminal() {
    let (e, _, now) = load("small-session");
    let snap = e.snapshot();
    let mut app = App {
        help: true,
        ..App::default()
    };
    insta::assert_snapshot!("help-100x30", render(&mut app, &snap, now, 100, 30));
    let mut app = App::default();
    assert_eq!(
        render(&mut app, &snap, now, 30, 8).lines().next(),
        Some("terminal too small for antfarm")
    );
}

#[test]
fn keys_drill_filter_and_back() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    let (mut e, id, now) = load("live-tail");
    register(&mut e, &id, "waiting", now);
    let snap = e.snapshot();
    let mut app = App::default();
    app.handle_key(key(KeyCode::Enter), &snap, now);
    assert_eq!(
        app.screen(),
        &Screen::Session {
            session: id.clone()
        }
    );
    app.focus = Focus::Calls;
    app.handle_key(key(KeyCode::Enter), &snap, now);
    assert!(matches!(app.screen(), Screen::Call { .. }));
    app.handle_key(key(KeyCode::Esc), &snap, now);
    app.handle_key(key(KeyCode::Esc), &snap, now);
    assert_eq!(app.screen(), &Screen::Machine);
    for c in "nomatch".chars() {
        if c == 'n' {
            app.handle_key(key(KeyCode::Char('/')), &snap, now);
        }
        app.handle_key(key(KeyCode::Char(c)), &snap, now);
    }
    app.handle_key(key(KeyCode::Enter), &snap, now);
    assert_eq!(app.prefs.filter, "nomatch");
    assert!(view::table_rows(&snap, &app.prefs, false, now).is_empty());
    app.handle_key(key(KeyCode::Esc), &snap, now);
    assert!(app.prefs.filter.is_empty());
    assert_eq!(
        app.handle_key(key(KeyCode::Char('g')), &snap, now),
        Some(super::app::Action::GoTo(id.clone()))
    );
    app.handle_key(key(KeyCode::Char('z')), &snap, now);
    assert!(
        view::needs_you(&snap, &app.prefs, now).is_empty(),
        "snoozed"
    );
    app.handle_key(key(KeyCode::Char('q')), &snap, now);
    assert!(app.quit);
}

fn lanes_screen(app: &mut App, snap: &Snapshot, id: &SessionId, now: Time) {
    app.stack.push(Screen::Machine);
    app.stack.truncate(1);
    app.stack.push(Screen::Session {
        session: id.clone(),
    });
    app.lanes = super::app::LanesState::default();
    app.stack.push(Screen::Lanes {
        session: id.clone(),
    });
    let _ = (snap, now);
}

#[test]
fn swimlane_screens() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    for (name, waiting) in [
        ("live-tail", None),
        ("small-session", None),
        ("live-tail", Some("waiting")),
    ] {
        let (mut e, id, now) = load(name);
        if let Some(st) = waiting {
            register(&mut e, &id, st, now);
        }
        let snap = e.snapshot();
        let tag = waiting
            .map(|s| format!("{name}-{s}"))
            .unwrap_or_else(|| name.to_owned());
        let mut app = App::default();
        lanes_screen(&mut app, &snap, &id, now);
        insta::assert_snapshot!(
            format!("{tag}-lanes-15m-120x40"),
            render(&mut app, &snap, now, 120, 40)
        );
        app.handle_key(key(KeyCode::Char('f')), &snap, now);
        insta::assert_snapshot!(
            format!("{tag}-lanes-fit-120x40"),
            render(&mut app, &snap, now, 120, 40)
        );
        // Back to 15 minutes, then rewind the playhead by ten calls.
        app.lanes = super::app::LanesState::default();
        render(&mut app, &snap, now, 120, 40);
        for _ in 0..10 {
            app.handle_key(key(KeyCode::Char(',')), &snap, now);
        }
        assert!(!app.lanes.follow);
        insta::assert_snapshot!(
            format!("{tag}-lanes-rewound-120x40"),
            render(&mut app, &snap, now, 120, 40)
        );
        insta::assert_snapshot!(
            format!("{tag}-lanes-80x24"),
            render(&mut app, &snap, now, 80, 24)
        );
    }
}

#[test]
fn swimlane_keys() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    let (e, id, now) = load("live-tail");
    let snap = e.snapshot();
    let mut app = App::default();
    // L from the machine screen opens the selected session's lanes.
    app.handle_key(key(KeyCode::Char('l')), &snap, now);
    assert_eq!(
        app.screen(),
        &Screen::Lanes {
            session: id.clone()
        }
    );
    render(&mut app, &snap, now, 120, 40);
    let s = &snap.machine.sessions[&id];
    let (w0, p0) = app.lanes.window(s, now);
    assert_eq!(p0, now, "follow puts the playhead on now");
    app.handle_key(key(KeyCode::Left), &snap, now);
    let (_, p1) = app.lanes.window(s, now);
    assert!(!app.lanes.follow && p1 < now);
    app.handle_key(key(KeyCode::Char('+')), &snap, now);
    let (w1, p2) = app.lanes.window(s, now);
    assert!(w1.secs() < w0.secs(), "zoomed in");
    assert!(p2 >= w1.start && p2 <= w1.end, "the playhead stays in view");
    app.handle_key(key(KeyCode::Char('-')), &snap, now);
    app.handle_key(key(KeyCode::Char('-')), &snap, now);
    let (w2, _) = app.lanes.window(s, now);
    assert!(w2.secs() > w0.secs(), "zoomed out");
    app.handle_key(key(KeyCode::PageUp), &snap, now);
    let (w3, _) = app.lanes.window(s, now);
    assert!(w3.end < w2.end, "panned back");
    app.handle_key(key(KeyCode::End), &snap, now);
    assert!(app.lanes.follow);
    assert_eq!(app.lanes.window(s, now).1, now);
    // Enter on the selected lane opens that agent; Esc comes back.
    app.handle_key(key(KeyCode::Down), &snap, now);
    app.handle_key(key(KeyCode::Enter), &snap, now);
    assert!(matches!(app.screen(), Screen::Agent { .. }));
    app.handle_key(key(KeyCode::Esc), &snap, now);
    assert!(matches!(app.screen(), Screen::Lanes { .. }));
    // Tab to the calls panel, Enter opens a call.
    app.handle_key(key(KeyCode::Tab), &snap, now);
    app.handle_key(key(KeyCode::Enter), &snap, now);
    assert!(matches!(app.screen(), Screen::Call { .. }));
    // Sizes from tiny to large never panic.
    app.stack.truncate(1);
    app.handle_key(key(KeyCode::Char('l')), &snap, now);
    for (w, h) in [
        (40, 10),
        (41, 11),
        (60, 15),
        (79, 23),
        (80, 24),
        (100, 30),
        (200, 60),
    ] {
        render(&mut app, &snap, now, w, h);
    }
}
