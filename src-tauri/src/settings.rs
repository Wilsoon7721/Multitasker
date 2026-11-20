use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// This computer's preferences, kept beside the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Watch the window in front and linked files to tell which task is being worked on.
    pub detection: bool,
    /// Where tasks and MultiTasker's own state are shared with other computers.
    pub sync: SyncTarget,
}

impl Default for Settings {
    fn default() -> Self {
        Self { detection: true, sync: SyncTarget::Off }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncTarget {
    #[default]
    Off,
    /// A folder a sync client (OneDrive, Google Drive, Dropbox) carries between computers.
    Folder { path: String },
}

const FILE: &str = "settings.json";

pub fn load(data_dir: &Path) -> Settings {
    std::fs::read_to_string(data_dir.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Loads, changes and saves the settings, one caller at a time so concurrent changes don't overwrite each other.
pub fn update(data_dir: &Path, change: impl FnOnce(&mut Settings)) -> std::io::Result<Settings> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut settings = load(data_dir);
    change(&mut settings);
    save(data_dir, &settings)?;
    Ok(settings)
}

fn save(data_dir: &Path, settings: &Settings) -> std::io::Result<()> {
    std::fs::write(data_dir.join(FILE), serde_json::to_string_pretty(settings)?)
}
