use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use multitasker_core::{SettingsStore, SharedSettings, Stamp};
use multitasker_sync::Remote;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::{self, Settings};
use crate::AppState;

/// The settings that follow a person between computers; the rest (sign-ins, sync, starting with Windows, project
/// commands) belong to each computer.
pub const SHARED: &[&str] = &[
    "detection",
    "reminders",
    "widget_opacity",
    "pause_after_away",
    "auto_switch",
    "auto_start_after_secs",
    "widget_on_top",
    "switcher_shortcut",
];
/// Today's plan, which follows too.
const PLAN: &str = "plan";

/// Tells every window which shared settings just arrived from another computer.
pub const SETTINGS_SYNCED: &str = "settings-synced";
/// Tells every window the day's plan changed, here or elsewhere.
pub const PLAN_CHANGED: &str = "plan-changed";

/// This computer's copy, with when each setting last changed anywhere.
const FILE: &str = "shared-settings.json";
/// Where each computer leaves its copy in a sync location; each writes only its own.
const REMOTE_DIR: &str = "multitasker/v1/settings/";
/// How often to look for other computers' changes in a task list that keeps the settings.
const INTERVAL: Duration = Duration::from_secs(60);

static FILE_LOCK: Mutex<()> = Mutex::new(());
static WOKEN: Mutex<bool> = Mutex::new(false);
static WAKE: Condvar = Condvar::new();
static STATUS: Mutex<Option<SettingsSync>> = Mutex::new(None);

/// How sharing settings is going, for Settings.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsSync {
    /// What carries them: the task list's name, or "your sync location".
    pub via: String,
    pub last_synced: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

pub fn status() -> Option<SettingsSync> {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_status(via: &str, outcome: Result<(), String>) {
    let mut status = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    let last_synced = status.as_ref().filter(|s| s.via == via).and_then(|s| s.last_synced);
    *status = Some(match outcome {
        Ok(()) => SettingsSync { via: via.to_owned(), last_synced: Some(Utc::now()), error: None },
        Err(e) => SettingsSync { via: via.to_owned(), last_synced, error: Some(e) },
    });
}

/// Asks for a round now, after a change here.
pub fn wake() {
    *WOKEN.lock().unwrap_or_else(|e| e.into_inner()) = true;
    WAKE.notify_one();
}

/// Sleeps until woken or the interval passes; returns whether something here changed.
fn sleep() -> bool {
    let woken = WOKEN.lock().unwrap_or_else(|e| e.into_inner());
    let (mut woken, _) = WAKE.wait_timeout_while(woken, INTERVAL, |w| !*w).unwrap_or_else(|e| e.into_inner());
    std::mem::replace(&mut *woken, false)
}

fn path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(FILE)
}

fn read(data_dir: &Path) -> Option<SharedSettings> {
    std::fs::read_to_string(path(data_dir)).ok().map(|text| SharedSettings::from_json(&text))
}

fn write(data_dir: &Path, doc: &SharedSettings) {
    if let Err(e) = std::fs::write(path(data_dir), doc.to_json()) {
        eprintln!("couldn't save shared settings: {e}");
    }
}

fn device_id(data_dir: &Path) -> String {
    crate::device::load_or_create(data_dir).map(|d| d.id).unwrap_or_else(|_| "this-computer".into())
}

fn value_of(settings: &Settings, key: &str) -> Value {
    serde_json::to_value(settings).ok().and_then(|v| v.get(key).cloned()).unwrap_or(Value::Null)
}

/// Starts this computer's copy from its settings the first time. They're stamped with when settings.json was last
/// saved, so a fresh install's defaults (no file yet) lose to every choice made on another computer.
pub fn ensure(data_dir: &Path) {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if path(data_dir).exists() {
        return;
    }
    let millis = std::fs::metadata(data_dir.join("settings.json"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    let settings = settings::load(data_dir);
    let device = device_id(data_dir);
    let mut doc = SharedSettings::default();
    for key in SHARED {
        doc.set(key, value_of(&settings, key), Stamp::new(millis, 0, &device));
    }
    write(data_dir, &doc);
}

/// Stamps the shared settings a change made here touched, so they reach the other computers.
pub fn record(data_dir: &Path, before: &Settings, after: &Settings) {
    let changed: Vec<(&str, Value)> = SHARED
        .iter()
        .map(|key| (*key, value_of(after, key)))
        .filter(|(key, value)| *value != value_of(before, key))
        .collect();
    set_here(data_dir, changed);
}

fn set_here(data_dir: &Path, values: Vec<(&str, Value)>) {
    if values.is_empty() {
        return;
    }
    {
        let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = read(data_dir).unwrap_or_default();
        let device = device_id(data_dir);
        for (key, value) in values {
            let stamp = Stamp::next(doc.latest(), Utc::now().timestamp_millis(), &device);
            doc.set(key, value, stamp);
        }
        write(data_dir, &doc);
    }
    wake();
}

/// Merges another copy into this computer's; returns the keys whose values changed and the merged copy.
fn merge_in(data_dir: &Path, theirs: &SharedSettings) -> (Vec<String>, SharedSettings) {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = read(data_dir).unwrap_or_default();
    let before = doc.clone();
    let changed = doc.merge(theirs);
    if doc != before {
        write(data_dir, &doc);
    }
    (changed, doc)
}

/// Puts settings that arrived from another computer to use here.
fn apply(app: &AppHandle, keys: &[String], doc: &SharedSettings) {
    if keys.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let old = settings::load(&state.data_dir);
    let now = settings::update_quiet(&state.data_dir, |s| {
        let Ok(mut value) = serde_json::to_value(&*s) else { return };
        for key in keys.iter().filter(|k| SHARED.contains(&k.as_str())) {
            if let Some(v) = doc.get(key) {
                value[key.as_str()] = v.clone();
            }
        }
        if let Ok(merged) = serde_json::from_value(value) {
            *s = merged;
        }
    });
    let Ok(now) = now else { return };

    for key in keys {
        match key.as_str() {
            "detection" => {
                state.detector.set_enabled(app, now.detection);
                crate::detection::sync_tray(app, now.detection);
            }
            "auto_start_after_secs" => state.detector.set_start_after(now.auto_start_after_secs),
            "auto_switch" => state.detector.set_auto_switch(now.auto_switch),
            "widget_on_top" => crate::on_top::set(now.widget_on_top.clone()),
            "widget_opacity" => {
                let _ = app.emit(crate::commands::WIDGET_APPEARANCE, now.widget_opacity);
            }
            // A hotkey another program holds on this computer keeps this computer's own instead.
            "switcher_shortcut" => {
                if crate::switcher::set_shortcut(app, now.switcher_shortcut.as_deref()).is_err() {
                    let _ = crate::switcher::set_shortcut(app, old.switcher_shortcut.as_deref());
                    let _ = settings::update_quiet(&state.data_dir, |s| s.switcher_shortcut = old.switcher_shortcut.clone());
                }
            }
            PLAN => {
                let _ = app.emit(PLAN_CHANGED, ());
            }
            _ => {}
        }
    }
    let _ = app.emit(SETTINGS_SYNCED, keys);
}

/// One round with a task list that keeps the settings: take its newer ones, and give it this computer's.
async fn round_with(app: &AppHandle, store: &dyn SettingsStore) -> Result<(), String> {
    let data_dir = app.state::<AppState>().data_dir.clone();
    let theirs = store.load_settings().await.map_err(|e| e.to_string())?;
    let (changed, merged) = merge_in(&data_dir, theirs.as_ref().unwrap_or(&SharedSettings::default()));
    apply(app, &changed, &merged);
    if theirs.as_ref() != Some(&merged) {
        store.save_settings(&merged).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One round through a sync location, run by the sync thread after tasks: read every computer's copy, merge, and
/// leave this one's.
pub fn sync_remote(app: &AppHandle, remote: &dyn Remote, device: &str) {
    let outcome = (|| -> Result<(), String> {
        let data_dir = app.state::<AppState>().data_dir.clone();
        let own = format!("{REMOTE_DIR}{device}.json");
        let mut theirs = SharedSettings::default();
        let mut stored = None;
        for key in remote.list(REMOTE_DIR).map_err(|e| e.to_string())? {
            if !key.ends_with(".json") {
                continue;
            }
            let Some(bytes) = remote.get(&key).map_err(|e| e.to_string())? else { continue };
            let doc = SharedSettings::from_json(&String::from_utf8_lossy(&bytes));
            if key == own {
                stored = Some(doc);
            } else {
                theirs.merge(&doc);
            }
        }
        let (changed, merged) = merge_in(&data_dir, &theirs);
        apply(app, &changed, &merged);
        if stored.as_ref() != Some(&merged) {
            remote.put(&own, merged.to_json().as_bytes()).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    set_status("your sync location", outcome);
}

/// Keeps settings in step with the task list in the background, when it's one that keeps them.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("shared-settings".into())
        .spawn(move || {
            let mut changed_here = true;
            loop {
                let state = app.state::<AppState>();
                let provider = state.provider();
                match provider.settings() {
                    Some(store) => {
                        let outcome = tauri::async_runtime::block_on(round_with(&app, store));
                        set_status(provider.display_name(), outcome);
                    }
                    // Otherwise they travel with the tasks through the sync location, if there is one.
                    None if changed_here => state.syncer.wake_soon(),
                    None => {}
                }
                changed_here = sleep();
            }
        })
        .expect("failed to start the shared settings thread");
}

/// The tasks picked as what a day is for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// The local date it's for, e.g. "2026-11-22"; a plan for another day counts as none.
    pub day: String,
    #[serde(default)]
    pub tasks: Vec<String>,
    /// Planning was turned down for the day.
    #[serde(default)]
    pub skipped: bool,
}

pub fn plan(data_dir: &Path) -> Option<Plan> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(data_dir)?.get(PLAN).and_then(|v| serde_json::from_value(v.clone()).ok())
}

pub fn set_plan(app: &AppHandle, plan: &Plan) {
    let data_dir = app.state::<AppState>().data_dir.clone();
    set_here(&data_dir, vec![(PLAN, serde_json::to_value(plan).unwrap_or(Value::Null))]);
    let _ = app.emit(PLAN_CHANGED, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("multitasker-shared-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_fresh_install_loses_to_choices_made_elsewhere() {
        let dir = temp_dir();
        ensure(&dir);
        let doc = read(&dir).unwrap();
        assert_eq!(doc.entries["widget_opacity"].stamp.millis(), 0);

        let mut elsewhere = SharedSettings::default();
        elsewhere.set("widget_opacity", serde_json::json!(70), Stamp::new(1, 0, "other"));
        let (changed, merged) = merge_in(&dir, &elsewhere);
        assert_eq!(changed, ["widget_opacity"]);
        assert_eq!(merged.get("widget_opacity"), Some(&serde_json::json!(70)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_shared_settings_changed_here_are_stamped() {
        let dir = temp_dir();
        ensure(&dir);
        let before = Settings::default();
        let after = Settings { widget_opacity: 50, start_with_windows: false, ..Settings::default() };
        record(&dir, &before, &after);
        let doc = read(&dir).unwrap();
        assert_eq!(doc.get("widget_opacity"), Some(&serde_json::json!(50)));
        assert!(doc.entries["widget_opacity"].stamp.millis() > 0);
        assert!(doc.get("start_with_windows").is_none(), "starting with Windows stays with each computer");
        let _ = std::fs::remove_dir_all(dir);
    }
}
