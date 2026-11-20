use std::path::Path;

use serde::{Deserialize, Serialize};

/// This computer's preferences, kept beside the database until settings sync arrives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Watch the window in front and linked files to tell which task is being worked on.
    pub detection: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { detection: true }
    }
}

const FILE: &str = "settings.json";

pub fn load(data_dir: &Path) -> Settings {
    std::fs::read_to_string(data_dir.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(data_dir: &Path, settings: &Settings) -> std::io::Result<()> {
    std::fs::write(data_dir.join(FILE), serde_json::to_string_pretty(settings)?)
}
