use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use multitasker_core::DeviceInfo;
use multitasker_sync::{sync_once, FolderRemote, Remote};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::commands::TASKS_CHANGED;
use crate::settings::{self, SyncTarget};
use crate::AppState;

/// Tells every window how sync is doing.
pub const SYNC_STATUS: &str = "sync-status";

/// How often to sync when nothing changes locally, to pick up other computers' changes.
const INTERVAL: Duration = Duration::from_secs(30);
/// After a local change, wait this long so a burst of edits goes up as one batch.
const SETTLE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub last_seen: DateTime<Utc>,
    pub this_device: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub target: SyncTarget,
    /// "off", "syncing", "ok" or "error".
    pub state: &'static str,
    pub last_synced: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub devices: Vec<DeviceView>,
}

pub struct Syncer {
    status: Mutex<SyncStatus>,
    woken: Mutex<bool>,
    wake: Condvar,
}

impl Syncer {
    pub fn new(target: SyncTarget) -> Arc<Self> {
        let state = if target == SyncTarget::Off { "off" } else { "syncing" };
        Arc::new(Self {
            status: Mutex::new(SyncStatus { target, state, last_synced: None, error: None, devices: Vec::new() }),
            woken: Mutex::new(false),
            wake: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, SyncStatus> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> SyncStatus {
        self.lock().clone()
    }

    /// Asks for a sync round now instead of at the next interval.
    pub fn wake(&self) {
        *self.woken.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake.notify_one();
    }

    /// Sleeps until woken or the interval passes; returns whether it was woken.
    fn sleep(&self) -> bool {
        let woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        let (mut woken, _) = self.wake.wait_timeout_while(woken, INTERVAL, |w| !*w).unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *woken, false)
    }

    fn update(&self, app: &AppHandle, f: impl FnOnce(&mut SyncStatus)) {
        let snapshot = {
            let mut status = self.lock();
            f(&mut status);
            status.clone()
        };
        let _ = app.emit(SYNC_STATUS, snapshot);
    }
}

/// Syncs in the background for as long as the app runs.
pub fn start(app: &AppHandle, syncer: Arc<Syncer>) {
    let waker = syncer.clone();
    let handle = app.clone();
    // Only local edits queue changes; merged remote ones don't, so this doesn't loop on its own syncs.
    app.listen_any(TASKS_CHANGED, move |_| {
        let state = handle.state::<AppState>();
        if state.db.pending_ops(1).is_ok_and(|p| !p.is_empty()) {
            waker.wake();
        }
    });

    let app = app.clone();
    std::thread::Builder::new()
        .name("sync".into())
        .spawn(move || loop {
            run_once(&app, &syncer);
            if syncer.sleep() {
                std::thread::sleep(SETTLE);
            }
        })
        .expect("failed to start the sync thread");
}

fn run_once(app: &AppHandle, syncer: &Syncer) {
    let state = app.state::<AppState>();
    let target = settings::load(&state.data_dir).sync;

    let remote: Box<dyn Remote> = match &target {
        SyncTarget::Off => {
            // Settings can say off while the database still logs changes (e.g. settings.json was removed).
            if state.db.sync_device().is_ok_and(|d| d.is_some()) {
                let _ = state.db.disable_sync();
            }
            syncer.update(app, |s| *s = SyncStatus { target, state: "off", last_synced: None, error: None, devices: Vec::new() });
            return;
        }
        SyncTarget::Folder { path } => match FolderRemote::new(path) {
            Ok(remote) => Box::new(remote),
            Err(e) => return fail(app, syncer, target.clone(), e.to_string()),
        },
    };

    syncer.update(app, |s| {
        s.target = target.clone();
        s.state = "syncing";
    });
    let me = DeviceInfo { id: state.device.id.clone(), name: state.device.name.clone(), last_seen: Utc::now() };
    match sync_once(&state.db, remote.as_ref(), &me) {
        Ok(report) => {
            if report.applied > 0 {
                let _ = app.emit(TASKS_CHANGED, ());
            }
            let devices = report
                .devices
                .into_iter()
                .map(|d| DeviceView { this_device: d.id == me.id, id: d.id, name: d.name, last_seen: d.last_seen })
                .collect();
            syncer.update(app, |s| {
                *s = SyncStatus { target, state: "ok", last_synced: Some(Utc::now()), error: None, devices }
            });
        }
        Err(e) => fail(app, syncer, target, e.to_string()),
    }
}

fn fail(app: &AppHandle, syncer: &Syncer, target: SyncTarget, error: String) {
    syncer.update(app, |s| {
        s.target = target;
        s.state = "error";
        s.error = Some(error);
    });
}

/// Checks a chosen folder exists and can be written to before syncing into it.
pub fn check_folder(path: &str) -> Result<(), String> {
    let dir = Path::new(path);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(format!("Can't find the folder '{path}'."));
    }
    let probe = dir.join(format!(".multitasker-write-test-{}", std::process::id()));
    std::fs::write(&probe, b"ok").map_err(|e| format!("Can't write to '{path}': {e}"))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// A sync client folder found on this computer, offered as a one-click choice.
#[derive(Debug, Clone, Serialize)]
pub struct FolderSuggestion {
    pub label: String,
    pub path: String,
}

pub fn folder_suggestions() -> Vec<FolderSuggestion> {
    let mut found: Vec<FolderSuggestion> = Vec::new();
    let mut add = |label: &str, path: std::path::PathBuf| {
        if path.is_dir() && !found.iter().any(|f| Path::new(&f.path) == path) {
            found.push(FolderSuggestion { label: label.into(), path: path.display().to_string() });
        }
    };

    for (var, label) in [("OneDriveConsumer", "OneDrive"), ("OneDriveCommercial", "OneDrive (work or school)"), ("OneDrive", "OneDrive")] {
        if let Ok(path) = std::env::var(var) {
            add(label, path.into());
        }
    }
    // Google Drive for desktop mounts as a drive letter (G: by default) with "My Drive" at its root.
    for letter in 'D'..='Z' {
        add("Google Drive", format!("{letter}:\\My Drive").into());
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        add("Dropbox", Path::new(&home).join("Dropbox"));
    }
    found
}
