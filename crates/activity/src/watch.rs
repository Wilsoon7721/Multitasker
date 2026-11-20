use std::collections::HashMap;
use std::path::PathBuf;

use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Watches linked files and folders, reporting paths as they're created or saved.
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    watched: HashMap<PathBuf, RecursiveMode>,
}

impl FileWatcher {
    pub fn new(on_save: impl Fn(PathBuf) + Send + 'static) -> notify::Result<Self> {
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let Ok(event) = event else { return };
            if is_save(&event.kind) {
                event.paths.into_iter().for_each(&on_save);
            }
        })?;
        Ok(Self { watcher, watched: HashMap::new() })
    }

    /// Watches exactly these paths (folders with everything inside), dropping any others. Missing paths are skipped.
    pub fn watch_only(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let wanted: HashMap<PathBuf, RecursiveMode> = paths
            .into_iter()
            .filter_map(|p| {
                let meta = std::fs::metadata(&p).ok()?;
                Some((p, if meta.is_dir() { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive }))
            })
            .collect();

        self.watched.retain(|path, mode| {
            let keep = wanted.get(path) == Some(mode);
            if !keep {
                let _ = self.watcher.unwatch(path);
            }
            keep
        });

        for (path, mode) in wanted {
            if !self.watched.contains_key(&path) && self.watcher.watch(&path, mode).is_ok() {
                self.watched.insert(path, mode);
            }
        }
    }
}

// Saves show up as creates, content changes or a rename onto the file (how Word and many editors save).
fn is_save(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Any | ModifyKind::Name(RenameMode::To | RenameMode::Both))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn reports_saves_inside_watched_folders() {
        let dir = std::env::temp_dir().join(format!("multitasker-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let (tx, rx) = mpsc::channel();
        let mut watcher = FileWatcher::new(move |p| {
            let _ = tx.send(p);
        })
        .unwrap();
        watcher.watch_only([dir.clone(), dir.join("missing")]);
        assert_eq!(watcher.watched.len(), 1);

        let file = dir.join("notes.txt");
        std::fs::write(&file, "hello").unwrap();
        let seen = rx.recv_timeout(Duration::from_secs(5)).expect("no event for the saved file");
        assert_eq!(seen.file_name(), file.file_name());

        watcher.watch_only([]);
        assert!(watcher.watched.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
