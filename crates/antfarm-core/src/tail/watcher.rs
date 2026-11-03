//! File-system change hints.
//!
//! notify events only say that something under a watched directory may have changed.
//! The tailers' own `stat` checks are the truth, and a timed poll covers events that
//! FSEvents coalesces or drops.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub struct WatchSet {
    watcher: Option<RecommendedWatcher>,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    watched: HashSet<PathBuf>,
    backend: &'static str,
}

impl WatchSet {
    /// Starts a native watcher, or a poll-only set when `poll_only` is true or the
    /// watcher cannot start.
    pub fn new(poll_only: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let watcher = if poll_only {
            None
        } else {
            notify::recommended_watcher(move |res| {
                let _ = tx.send(res);
            })
            .ok()
        };
        let backend = match &watcher {
            None => "poll",
            Some(_) if cfg!(target_os = "macos") => "fsevents",
            Some(_) if cfg!(target_os = "linux") => "inotify",
            Some(_) => "native",
        };
        Self {
            watcher,
            rx,
            watched: HashSet::new(),
            backend,
        }
    }

    pub fn backend(&self) -> &'static str {
        self.backend
    }

    pub fn is_native(&self) -> bool {
        self.watcher.is_some()
    }

    /// Watches one directory, not its subdirectories. Repeated calls are free.
    pub fn watch(&mut self, dir: &Path) {
        let Some(w) = self.watcher.as_mut() else {
            return;
        };
        if self.watched.contains(dir) || !dir.is_dir() {
            return;
        }
        if w.watch(dir, RecursiveMode::NonRecursive).is_ok() {
            self.watched.insert(dir.to_path_buf());
        }
    }

    /// Waits up to `timeout` for a change hint, then drains any others that arrive within
    /// a short settle window. Returns true when something changed.
    pub fn wait(&self, timeout: Duration) -> bool {
        match self.rx.recv_timeout(timeout) {
            Ok(_) => {
                std::thread::sleep(Duration::from_millis(30));
                while self.rx.try_recv().is_ok() {}
                true
            }
            Err(_) => false,
        }
    }
}
