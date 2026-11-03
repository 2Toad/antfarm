//! The antfarm desktop app: the same live model as the terminal UI, drawn in a webview,
//! with a menu bar tray and notifications for sessions that need the human.

pub mod screens;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use antfarm_core::claude::{ClaudeCodeSource, SourceConfig};
use antfarm_core::source::{Clock, Handle, Hub};
use antfarm_core::{Paths, SessionId, SessionStatus, Snapshot};
use antfarm_view::{Prefs, SortMode};
use screens::{Notify, Route};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Notify as Wake;

/// Views go out at most this often while sessions are busy.
const VIEW_EVERY: Duration = Duration::from_millis(250);
const SHORTCUT: &str = "CmdOrCtrl+Shift+A";

pub struct AppState {
    handle: Handle,
    ui: Mutex<Ui>,
    wake: Arc<Wake>,
    host: String,
}

/// What the person has chosen in the app, independent of the snapshot.
#[derive(Default)]
struct Ui {
    route: Option<Route>,
    prefs: Prefs,
    show_exited: bool,
    notify: HashMap<SessionId, Notify>,
}

fn host() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "machine".into())
}

#[tauri::command]
fn set_route(state: State<'_, AppState>, route: Route) {
    state.ui.lock().unwrap().route = Some(route);
    state.wake.notify_one();
}

#[derive(serde::Deserialize)]
struct PrefsIn {
    filter: Option<String>,
    sort: Option<String>,
    tree: Option<bool>,
    show_finished: Option<bool>,
    show_exited: Option<bool>,
}

#[tauri::command]
fn set_prefs(state: State<'_, AppState>, prefs: PrefsIn) {
    let mut ui = state.ui.lock().unwrap();
    if let Some(f) = prefs.filter {
        ui.prefs.filter = f;
    }
    if let Some(s) = prefs.sort {
        ui.prefs.sort = match s.as_str() {
            "cost" => SortMode::Cost,
            "name" => SortMode::Name,
            _ => SortMode::NeedsYou,
        };
    }
    if let Some(t) = prefs.tree {
        ui.prefs.tree = t;
    }
    if let Some(f) = prefs.show_finished {
        ui.prefs.show_finished = f;
    }
    if let Some(e) = prefs.show_exited {
        ui.show_exited = e;
    }
    drop(ui);
    state.wake.notify_one();
}

#[tauri::command]
fn go_to_terminal(state: State<'_, AppState>, session: SessionId) -> Result<String, String> {
    let snap = state.handle.snapshot();
    let s = snap
        .machine
        .sessions
        .get(&session)
        .ok_or("that session is gone")?;
    antfarm_core::actions::go(s)
}

#[tauri::command]
fn copy(text: String) -> Result<String, String> {
    antfarm_core::actions::copy(&text)
}

#[tauri::command]
fn snooze(state: State<'_, AppState>, session: SessionId) -> String {
    let until = chrono::Utc::now() + chrono::Duration::minutes(10);
    state
        .ui
        .lock()
        .unwrap()
        .prefs
        .snoozed
        .insert(session, until);
    state.wake.notify_one();
    "snoozed for 10 minutes".into()
}

#[tauri::command]
fn mute(state: State<'_, AppState>, session: SessionId) -> String {
    let mut ui = state.ui.lock().unwrap();
    let msg = if ui.prefs.muted.remove(&session) {
        "unmuted"
    } else {
        ui.prefs.muted.insert(session);
        "muted: dimmed and sorted last"
    };
    drop(ui);
    state.wake.notify_one();
    msg.into()
}

#[tauri::command]
fn set_notify(state: State<'_, AppState>, session: SessionId, notify: Notify) {
    state.ui.lock().unwrap().notify.insert(session, notify);
    state.wake.notify_one();
}

/// Builds and sends the current route's view whenever the model or the route changes,
/// at most every [`VIEW_EVERY`].
async fn view_loop(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut snapshots = state.handle.snapshots();
    let mut last_status: HashMap<SessionId, SessionStatus> = HashMap::new();
    let mut tray_key = String::new();
    loop {
        let snap = snapshots.borrow_and_update().clone();
        let now = chrono::Utc::now();
        let (event, waiting_alerts) = {
            let ui = state.ui.lock().unwrap();
            let header = screens::header(&snap, &ui.prefs, &state.host, now);
            let body = ui
                .route
                .as_ref()
                .map(|r| screens::body(&snap, r, &ui.prefs, &ui.notify, ui.show_exited, now));
            let alerts = transitions(&snap, &mut last_status, &ui.notify);
            (
                body.map(|body| screens::ViewEvent {
                    seq: snap.seq,
                    now,
                    header,
                    body,
                }),
                alerts,
            )
        };
        if let Some(e) = &event {
            let _ = app.emit("view", e);
            // Debug aid: the latest view as JSON, for checking the pipeline without the UI.
            if let Some(path) = std::env::var_os("ANTFARM_VIEW_DUMP")
                && let Ok(json) = serde_json::to_vec_pretty(e)
            {
                let tmp = std::path::PathBuf::from(&path).with_extension("tmp");
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(tmp, &path);
                }
            }
            let key = tray_signature(e);
            if key != tray_key {
                tray_key = key;
                update_tray(&app, e);
            }
        }
        for (title, body) in waiting_alerts {
            let _ = app.notification().builder().title(title).body(body).show();
        }
        tokio::select! {
            _ = snapshots.changed() => {}
            _ = state.wake.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        tokio::time::sleep(VIEW_EVERY).await;
    }
}

/// Notifications for sessions that just started waiting, or just finished, when the
/// person asked for them. The first sight of a session never notifies.
fn transitions(
    snap: &Snapshot,
    last: &mut HashMap<SessionId, SessionStatus>,
    notify: &HashMap<SessionId, Notify>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in snap.machine.sessions.values() {
        let prev = last.insert(s.id.clone(), s.status);
        let Some(prev) = prev else { continue };
        if prev == s.status {
            continue;
        }
        let want = notify.get(&s.id).copied().unwrap_or_default();
        let name = antfarm_view::session_name(s);
        match s.status {
            SessionStatus::Waiting(kind) if want.waiting && !prev.needs_human() => {
                let (tool, target) = antfarm_view::session_current(s);
                let what = match kind {
                    antfarm_core::WaitingKind::Question => "has a question",
                    antfarm_core::WaitingKind::Permission => "needs permission",
                };
                out.push((
                    format!("{name} {what}"),
                    format!("{tool} {target}").trim().to_owned(),
                ));
            }
            SessionStatus::Idle
                if want.finished
                    && matches!(prev, SessionStatus::Working | SessionStatus::Thinking) =>
            {
                out.push((
                    format!("{name} finished"),
                    "Ready for your next prompt.".into(),
                ));
            }
            _ => {}
        }
    }
    out
}

/// What the tray shows, so it is only rebuilt when that changes.
fn tray_signature(e: &screens::ViewEvent) -> String {
    let waiting: Vec<String> = e
        .header
        .needs_you
        .iter()
        .map(|n| n.session.to_string())
        .collect();
    let sessions: Vec<String> = e
        .header
        .sessions
        .iter()
        .map(|s| format!("{}{}{:?}", s.id, s.tool, s.class))
        .collect();
    format!("{waiting:?}{sessions:?}")
}

fn update_tray(app: &AppHandle, e: &screens::ViewEvent) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    let n = e.header.needs_you.len();
    let _ = tray.set_title(if n > 0 { Some(n.to_string()) } else { None });
    if let Ok(menu) = tray_menu(app, e) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn tray_menu(app: &AppHandle, e: &screens::ViewEvent) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let n = e.header.needs_you.len();
    let header = if n == 0 {
        "Nothing is waiting for you".to_owned()
    } else {
        format!("{n} waiting for you")
    };
    menu.append(&MenuItem::with_id(
        app,
        "noop",
        header,
        false,
        None::<&str>,
    )?)?;
    for w in &e.header.needs_you {
        let kind = match w.kind {
            antfarm_core::WaitingKind::Permission => "permission",
            antfarm_core::WaitingKind::Question => "question",
        };
        let label = format!(
            "◆ {}   {kind} · {}   {}",
            w.name,
            antfarm_view::fmt::truncate(&w.request, 40),
            w.age.map(antfarm_view::fmt::dur).unwrap_or_default()
        );
        menu.append(&MenuItem::with_id(
            app,
            format!("go:{}", w.session),
            label,
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let working: Vec<_> = e
        .header
        .sessions
        .iter()
        .filter(|s| s.class == antfarm_view::Class::Working)
        .collect();
    menu.append(&MenuItem::with_id(
        app,
        "noop2",
        format!("Working · {} sessions", working.len()),
        false,
        None::<&str>,
    )?)?;
    for s in working {
        let extra = if s.agents.is_empty() {
            String::new()
        } else {
            format!(" +{}", s.agents.len())
        };
        menu.append(&MenuItem::with_id(
            app,
            format!("open:{}", s.id),
            format!("● {}{extra}   {}", s.name, s.tool),
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "show",
        "Open antfarm",
        true,
        Some(SHORTCUT),
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit antfarm",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    if let Some(session) = id.strip_prefix("go:") {
        let snap = app.state::<AppState>().handle.snapshot();
        if let Some(s) = snap.machine.sessions.get(session) {
            let _ = antfarm_core::actions::go(s);
        }
    } else if let Some(session) = id.strip_prefix("open:") {
        show_window(app);
        let _ = app.emit(
            "navigate",
            serde_json::json!({ "screen": "session", "id": session }),
        );
    } else if id == "show" {
        show_window(app);
    } else if id == "quit" {
        app.exit(0);
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        show_window(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            set_route,
            set_prefs,
            go_to_terminal,
            copy,
            snooze,
            mute,
            set_notify
        ])
        .setup(|app| {
            let source = ClaudeCodeSource::new(SourceConfig::new(Paths::from_env()));
            let handle =
                tauri::async_runtime::block_on(async { Hub::spawn(Box::new(source), Clock::Wall) });
            app.manage(AppState {
                handle,
                ui: Mutex::new(Ui {
                    route: Some(Route::Machine),
                    prefs: Prefs {
                        tree: true,
                        ..Prefs::default()
                    },
                    ..Ui::default()
                }),
                wake: Arc::new(Wake::new()),
                host: host(),
            });
            let icon = app.default_window_icon().cloned().ok_or("no app icon")?;
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .tooltip("antfarm")
                .menu(&Menu::new(app.handle())?)
                .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
                .build(app)?;
            let _ = app.global_shortcut().register(SHORTCUT);
            tauri::async_runtime::spawn(view_loop(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps antfarm in the menu bar.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("antfarm failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<AppState>().handle.stop();
            }
        });
}
