use std::path::Path;

use serde::{Deserialize, Serialize};

/// This computer's identity, made once and kept in the app data folder so it survives restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
}

pub fn load_or_create(data_dir: &Path) -> std::io::Result<Device> {
    let path = data_dir.join("device.json");

    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(device) = serde_json::from_str::<Device>(&text) {
            return Ok(device);
        }
    }

    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "This computer".into());
    let device = Device { id: uuid::Uuid::new_v4().to_string(), name };

    std::fs::write(&path, serde_json::to_string_pretty(&device)?)?;
    Ok(device)
}
