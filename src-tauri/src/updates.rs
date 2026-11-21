use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

/// Tells every window an update has been downloaded and is ready to install.
pub const UPDATE_READY: &str = "update-ready";
/// The argument Windows starts MultiTasker with at sign-in, so it opens quietly to the tray and widget.
pub const AUTOSTART_ARG: &str = "--autostart";

/// The first check waits a little, so starting up stays quick.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

/// An update downloaded and waiting for a restart.
struct Ready {
    update: Update,
    bytes: Vec<u8>,
}

static READY: Mutex<Option<Ready>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    /// The version downloaded and ready, if there is one.
    pub ready: Option<String>,
    /// Whether this build updates itself; development builds don't.
    pub enabled: bool,
}

pub fn info(app: &AppHandle) -> UpdateInfo {
    let ready = READY.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.update.version.clone());
    UpdateInfo { current: app.package_info().version.to_string(), ready, enabled: enabled() }
}

/// Only installed builds update themselves or start with Windows; a development build would register itself instead.
pub fn enabled() -> bool {
    !cfg!(debug_assertions)
}

/// Whether Windows started MultiTasker at sign-in.
pub fn started_at_sign_in() -> bool {
    std::env::args().any(|a| a == AUTOSTART_ARG)
}

/// Makes starting with Windows match the setting.
pub fn apply_autostart(app: &AppHandle, on: bool) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    let launcher = app.autolaunch();
    let result = if on { launcher.enable() } else { launcher.disable() };
    result.map_err(|e| e.to_string())
}

/// Checks for updates now and then, downloading one in the background when there is one.
pub fn start(app: &AppHandle) {
    if !enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            if let Err(e) = check(&app).await {
                eprintln!("couldn't check for updates: {e}");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Looks for a newer version and downloads it; returns whether one is ready.
pub async fn check(app: &AppHandle) -> Result<bool, String> {
    if READY.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        return Ok(true);
    }
    let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    let version = update.version.clone();
    *READY.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ready { update, bytes });
    let _ = app.emit(UPDATE_READY, &version);
    let _ = app
        .notification()
        .builder()
        .title(format!("MultiTasker {version} is ready"))
        .body("Restart it from Settings or the tray to update.")
        .show();
    Ok(true)
}

/// Installs the downloaded update and starts the new version.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let ready = READY.lock().unwrap_or_else(|e| e.into_inner()).take().ok_or("No update is ready yet.")?;
    ready.update.install(ready.bytes).map_err(|e| e.to_string())?;
    app.restart()
}
