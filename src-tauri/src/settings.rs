use std::path::Path;
use std::sync::Mutex;

use multitasker_sync::S3Config;
use serde::{Deserialize, Serialize};

/// This computer's preferences, kept beside the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Watch the window in front and linked files to tell which task is being worked on.
    pub detection: bool,
    /// Where tasks and MultiTasker's own state are shared with other computers.
    pub sync: SyncTarget,
    /// Notifications for tasks with a due time.
    pub reminders: Reminders,
    /// Where tasks come from.
    pub tasks: TaskSource,
    /// How opaque the widget's background is, in percent; its text always stays solid.
    pub widget_opacity: u8,
    /// Minutes nobody is at the computer before its in-progress tasks pause, if ever.
    pub pause_after_away: Option<u32>,
}

/// The widget's background never goes fully clear, so its text stays readable over anything.
pub const MIN_WIDGET_OPACITY: u8 = 30;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskSource {
    /// The SQLite file on this computer (and whatever sync carries it to).
    #[default]
    Local,
    /// A JavaDashboard calendar, signed in with WilsoonID.
    JavaDashboard { account: String },
    /// Todoist, with a personal API token kept in the credential store.
    Todoist { account: String },
    /// A Notion database (its data source), with an integration token kept in the credential store.
    Notion { account: String, database: String, name: String },
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            detection: true,
            sync: SyncTarget::Off,
            reminders: Reminders::default(),
            tasks: TaskSource::Local,
            widget_opacity: 94,
            pause_after_away: Some(15),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reminders {
    /// How long before a due time to remind, if at all.
    pub minutes_before: Option<u32>,
    /// Remind again when it's due.
    pub at_due_time: bool,
}

impl Default for Reminders {
    fn default() -> Self {
        Self { minutes_before: Some(15), at_due_time: true }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncTarget {
    #[default]
    Off,
    /// A folder a sync client (OneDrive, Google Drive, Dropbox) carries between computers.
    Folder { path: String },
    /// An S3-compatible bucket; its secret key lives in the system's credential store, not here.
    S3(S3Config),
    /// Google Drive's hidden app-data folder, signed in directly; the sign-in lives in the credential store.
    GoogleDrive {
        account: String,
        /// A client ID entered in Settings instead of the one this build was made with.
        #[serde(default)]
        client_id: Option<String>,
    },
    /// OneDrive's app folder, signed in directly with a personal Microsoft account.
    OneDrive {
        account: String,
        #[serde(default)]
        client_id: Option<String>,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sync_targets_in_the_shape_the_ui_sends() {
        let s3: SyncTarget = serde_json::from_str(
            r#"{"kind":"s3","provider":"r2","endpoint":null,"region":"auto","bucket":"b","prefix":null,"access_key_id":"k"}"#,
        )
        .unwrap();
        assert!(matches!(s3, SyncTarget::S3(ref c) if c.bucket == "b" && c.region == "auto"));
        assert_eq!(serde_json::to_value(&s3).unwrap()["kind"], "s3");

        let folder: SyncTarget = serde_json::from_str(r#"{"kind":"folder","path":"C:\\Sync"}"#).unwrap();
        assert_eq!(folder, SyncTarget::Folder { path: r"C:\Sync".into() });

        let drive: SyncTarget = serde_json::from_str(r#"{"kind":"google_drive","account":"me@gmail.com"}"#).unwrap();
        assert_eq!(drive, SyncTarget::GoogleDrive { account: "me@gmail.com".into(), client_id: None });
        let one: SyncTarget = serde_json::from_str(r#"{"kind":"one_drive","account":"me@outlook.com","client_id":"id"}"#).unwrap();
        assert!(matches!(one, SyncTarget::OneDrive { client_id: Some(ref c), .. } if c == "id"));

        let old: Settings = serde_json::from_str(r#"{"detection":false}"#).unwrap();
        assert_eq!((old.detection, old.sync), (false, SyncTarget::Off), "settings from before sync still load");
        assert_eq!(old.widget_opacity, 94);
    }
}
