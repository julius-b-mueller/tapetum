//! Reloads a show whenever BlinderKitten saves it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::{Event, RecursiveMode, Watcher};

use crate::{Error, Show};

/// Keeps the underlying OS watcher alive; drop it to stop watching.
pub struct ShowWatcher {
    _watcher: notify::RecommendedWatcher,
    pub updates: Receiver<Result<Show, Error>>,
}

/// Saves are debounced: JUCE writes to a temp file and moves it into place,
/// which shows up as several events in quick succession.
pub fn watch(path: impl AsRef<Path>, debounce: Duration) -> notify::Result<ShowWatcher> {
    let path: PathBuf = path.as_ref().canonicalize().unwrap_or_else(|_| path.as_ref().to_path_buf());
    let file_name = path.file_name().map(|n| n.to_owned());
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));

    let (raw_tx, raw_rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(ev) = res
            && ev.paths.iter().any(|p| p.file_name() == file_name.as_deref())
            && (ev.kind.is_modify() || ev.kind.is_create())
        {
            let _ = raw_tx.send(());
        }
    })?;
    // Watch the directory, not the file: an atomic replace would orphan a file watch.
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;

    let (tx, updates) = mpsc::channel();
    std::thread::Builder::new()
        .name("olga-watch".into())
        .spawn(move || {
            while raw_rx.recv().is_ok() {
                while raw_rx.recv_timeout(debounce).is_ok() {}
                if tx.send(crate::load(&path)).is_err() {
                    break;
                }
            }
        })
        .expect("spawn watcher thread");

    Ok(ShowWatcher { _watcher: watcher, updates })
}
