use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Result, TaskId};

/// Identifies one computer running MultiTasker, so state can say where something is happening.
pub type DeviceId = String;

/// One task marked as in progress, and where.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusEntry {
    pub provider: String,
    pub task_id: TaskId,
    pub device: DeviceId,
    pub started_at: DateTime<Utc>,
}

/// Where MultiTasker keeps its own state about tasks, apart from the task provider.
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Every in-progress entry, across all devices.
    async fn focus(&self) -> Result<Vec<FocusEntry>>;

    /// Marks a task in progress on `device`. Doing it again keeps the original start time.
    async fn start(&self, provider: &str, task_id: &TaskId, device: &DeviceId) -> Result<FocusEntry>;

    /// Clears a task's in-progress mark on every device.
    async fn stop(&self, provider: &str, task_id: &TaskId) -> Result<()>;
}
