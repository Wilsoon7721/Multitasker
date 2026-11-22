use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use multitasker_core::DeviceInfo;
use multitasker_sync::{sync_once, FolderRemote, GoogleDriveRemote, OneDriveRemote, Remote, S3Config, S3Remote, TokenSource};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::cloud::Cloud;
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
    /// "off", "paused", "syncing", "ok" or "error".
    pub state: &'static str,
    pub last_synced: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub devices: Vec<DeviceView>,
    /// The app that carries a shared folder between computers, when it can be told from the path.
    pub client: Option<SyncClient>,
}

/// A folder only reaches other computers while its sync app runs; MultiTasker can only see the local folder.
#[derive(Debug, Clone, Serialize)]
pub struct SyncClient {
    pub name: &'static str,
    /// `None` when running programs can't be listed.
    pub running: Option<bool>,
}

pub struct Syncer {
    status: Mutex<SyncStatus>,
    /// The signed-in service's tokens, kept between rounds so the access token isn't renewed every time.
    tokens: Mutex<Option<(SyncTarget, Arc<TokenSource>)>>,
    woken: Mutex<Wake>,
    wake: Condvar,
}

/// Why the sync thread woke: local edits settle for a moment first; things done on purpose run at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Wake {
    None,
    Soon,
    Now,
}

impl Syncer {
    pub fn new(target: SyncTarget) -> Arc<Self> {
        let state = if target == SyncTarget::Off { "off" } else { "syncing" };
        Arc::new(Self {
            status: Mutex::new(SyncStatus {
                target,
                state,
                last_synced: None,
                error: None,
                devices: Vec::new(),
                client: None,
            }),
            woken: Mutex::new(Wake::None),
            wake: Condvar::new(),
            tokens: Mutex::new(None),
        })
    }

    /// Hands over tokens from a fresh sign-in, so the first round doesn't need to renew them.
    pub fn use_tokens(&self, target: SyncTarget, tokens: Arc<TokenSource>) {
        *self.tokens.lock().unwrap_or_else(|e| e.into_inner()) = Some((target, tokens));
    }

    fn tokens_for(&self, target: &SyncTarget, cloud: Cloud, client_id: Option<&str>) -> Result<Arc<TokenSource>, String> {
        let mut cached = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, tokens)) = cached.as_ref() {
            if t == target {
                return Ok(tokens.clone());
            }
        }
        let tokens = cloud.tokens(cloud.app(client_id)?)?;
        *cached = Some((target.clone(), tokens.clone()));
        Ok(tokens)
    }

    fn lock(&self) -> MutexGuard<'_, SyncStatus> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> SyncStatus {
        self.lock().clone()
    }

    /// Asks for a sync round right away, for things the person did on purpose.
    pub fn wake(&self) {
        self.raise(Wake::Now);
    }

    /// Asks for a sync round shortly, after local edits have had a moment to settle.
    pub fn wake_soon(&self) {
        self.raise(Wake::Soon);
    }

    fn raise(&self, level: Wake) {
        let mut woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        *woken = (*woken).max(level);
        self.wake.notify_one();
    }

    /// Sleeps until woken or the interval passes; returns why it woke.
    fn sleep(&self) -> Wake {
        let woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        let (mut woken, _) =
            self.wake.wait_timeout_while(woken, INTERVAL, |w| *w == Wake::None).unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *woken, Wake::None)
    }

    /// Shows a new location straight away, before its first round finishes, so the UI never shows the old one.
    pub fn retarget(&self, app: &AppHandle, target: SyncTarget) {
        let client = match &target {
            SyncTarget::Folder { path } => folder_client(path),
            _ => None,
        };
        let state = if target == SyncTarget::Off { "off" } else { "syncing" };
        self.update(app, |s| *s = SyncStatus { target, state, last_synced: None, error: None, devices: Vec::new(), client });
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
            waker.wake_soon();
        }
    });

    let app = app.clone();
    std::thread::Builder::new()
        .name("sync".into())
        .spawn(move || loop {
            run_once(&app, &syncer);
            if syncer.sleep() == Wake::Soon {
                std::thread::sleep(SETTLE);
            }
        })
        .expect("failed to start the sync thread");
}

fn run_once(app: &AppHandle, syncer: &Syncer) {
    let state = app.state::<AppState>();
    let target = settings::load(&state.data_dir).sync;

    // A task list that keeps the extras itself needs no syncing; the target is kept for switching back.
    if state.provider().extras().is_some() {
        syncer.update(app, |s| {
            *s = SyncStatus { target, state: "paused", last_synced: None, error: None, devices: Vec::new(), client: None }
        });
        return;
    }

    let remote: Box<dyn Remote> = match &target {
        SyncTarget::Off => {
            // Settings can say off while the database still logs changes (e.g. settings.json was removed).
            if state.db.sync_device().is_ok_and(|d| d.is_some()) {
                let _ = state.db.disable_sync();
            }
            syncer.update(app, |s| {
                *s = SyncStatus { target, state: "off", last_synced: None, error: None, devices: Vec::new(), client: None }
            });
            return;
        }
        SyncTarget::Folder { path } => match FolderRemote::new(path) {
            Ok(remote) => Box::new(remote),
            Err(e) => return fail(app, syncer, target.clone(), e.to_string()),
        },
        SyncTarget::S3(config) => match s3_remote(config) {
            Ok(remote) => Box::new(remote),
            Err(e) => return fail(app, syncer, target.clone(), e),
        },
        SyncTarget::GoogleDrive { .. } | SyncTarget::OneDrive { .. } => {
            let (cloud, client_id) = Cloud::of(&target).expect("matched a cloud target");
            match syncer.tokens_for(&target, cloud, client_id) {
                Ok(tokens) => cloud_remote(cloud, tokens),
                Err(e) => return fail(app, syncer, target.clone(), e),
            }
        }
    };

    let client = match &target {
        SyncTarget::Folder { path } => folder_client(path),
        _ => None,
    };
    syncer.update(app, |s| {
        s.target = target.clone();
        s.state = "syncing";
        s.client = client.clone();
    });
    let me = DeviceInfo { id: state.device.id.clone(), name: state.device.name.clone(), last_seen: Utc::now() };
    // Tasks first, then handed-in work, whose copies can take longer to move.
    let outcome = sync_once(&state.db, remote.as_ref(), &me).map_err(|e| e.to_string()).and_then(|report| {
        multitasker_deliverables::sync(&state.deliverables, remote.as_ref(), &me.id, multitasker_provider_sqlite::PROVIDER_KEY)
            .map(|work| (report, work))
            .map_err(|e| e.to_string())
    });
    if outcome.is_ok() {
        crate::shared::sync_remote(app, remote.as_ref(), &me.id);
    }
    match outcome {
        Ok((report, work)) => {
            if report.applied > 0 || work.learned > 0 || work.downloaded > 0 {
                let _ = app.emit(TASKS_CHANGED, ());
            }
            let devices = report
                .devices
                .into_iter()
                .map(|d| DeviceView { this_device: d.id == me.id, id: d.id, name: d.name, last_seen: d.last_seen })
                .collect();
            syncer.update(app, |s| {
                *s = SyncStatus { target, state: "ok", last_synced: Some(Utc::now()), error: None, devices, client }
            });
        }
        Err(e) => fail(app, syncer, target, e),
    }
}

fn fail(app: &AppHandle, syncer: &Syncer, target: SyncTarget, error: String) {
    syncer.update(app, |s| {
        s.target = target;
        s.state = "error";
        s.error = Some(error);
    });
}

pub fn cloud_remote(cloud: Cloud, tokens: Arc<TokenSource>) -> Box<dyn Remote> {
    match cloud {
        Cloud::Google => Box::new(GoogleDriveRemote::new(tokens)),
        Cloud::Microsoft => Box::new(OneDriveRemote::new(tokens)),
    }
}

const KEYRING_SERVICE: &str = "MultiTasker";

fn keyring_user(config: &S3Config) -> String {
    let endpoint = config.endpoint.as_deref().filter(|e| !e.is_empty()).unwrap_or("aws");
    format!("s3:{endpoint}:{}", config.access_key_id)
}

fn keyring_entry(config: &S3Config) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_user(config)).map_err(|e| format!("Can't open the credential store: {e}"))
}

pub fn save_s3_secret(config: &S3Config, secret: &str) -> Result<(), String> {
    keyring_entry(config)?.set_password(secret).map_err(|e| format!("Can't save the secret key: {e}"))
}

pub fn forget_s3_secret(config: &S3Config) {
    if let Ok(entry) = keyring_entry(config) {
        let _ = entry.delete_credential();
    }
}

pub fn s3_remote(config: &S3Config) -> Result<S3Remote, String> {
    let secret = keyring_entry(config)?
        .get_password()
        .map_err(|_| "The secret key for this bucket isn't saved on this computer; enter it again in Settings.".to_owned())?;
    S3Remote::new(config.clone(), secret).map_err(|e| e.to_string())
}

/// Lists the bucket once, so wrong keys or names show up when saving rather than on the next sync.
pub fn check_s3(remote: &S3Remote) -> Result<(), String> {
    remote.list("multitasker/v1/devices/").map(|_| ()).map_err(|e| format!("Couldn't reach the bucket: {e}"))
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

/// Works out which sync app a folder belongs to, and whether that app is running right now.
fn folder_client(path: &str) -> Option<SyncClient> {
    let path = path.to_lowercase();
    let under = |var: &str| std::env::var(var).is_ok_and(|root| !root.is_empty() && path.starts_with(&root.to_lowercase()));
    let (name, exe) = if under("OneDrive") || under("OneDriveConsumer") || under("OneDriveCommercial") {
        ("OneDrive", "OneDrive.exe")
    } else if path.contains("\\my drive") || path.contains("\\shared drives") {
        ("Google Drive", "GoogleDriveFS.exe")
    } else if path.contains("\\dropbox") {
        ("Dropbox", "Dropbox.exe")
    } else {
        return None;
    };
    Some(SyncClient { name, running: multitasker_activity::is_process_running(exe) })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliberate_wakes_outrank_settling_ones() {
        let syncer = Syncer::new(SyncTarget::Off);
        syncer.wake_soon();
        syncer.wake();
        syncer.wake_soon();
        let started = std::time::Instant::now();
        assert_eq!(syncer.sleep(), Wake::Now);
        assert!(started.elapsed() < Duration::from_secs(1), "a pending wake returns at once");
    }
}
