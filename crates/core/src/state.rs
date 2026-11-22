use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Attachment, Result, TaskId};

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

/// A stretch of time spent on a task, from starting it to pausing or finishing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkSession {
    pub id: String,
    pub provider: String,
    pub task_id: TaskId,
    pub device: DeviceId,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// The task list counted it in the task's own total, so it's kept here only for the history of where time went.
    #[serde(default)]
    pub in_list: bool,
}

/// One step in a task's checklist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckItem {
    /// The task list's own id for it, or MultiTasker's for its own tasks.
    pub id: String,
    pub text: String,
    pub done: bool,
    pub added_at: DateTime<Utc>,
}

/// A note on where work on a task stopped, to pick it up again quickly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeftOff {
    pub text: String,
    pub at: DateTime<Utc>,
    pub device: DeviceId,
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

    /// Every attachment, oldest first.
    async fn attachments(&self) -> Result<Vec<Attachment>>;

    /// Stores an attachment, or returns the task's existing one with the same target.
    async fn add_attachment(&self, attachment: Attachment) -> Result<Attachment>;

    /// Removes one attachment. The file or link it points at is left alone.
    async fn remove_attachment(&self, id: &str) -> Result<()>;

    /// Notes that work on a task began on this computer; a task already being worked on keeps its first time.
    async fn begin_work(&self, provider: &str, task_id: &TaskId, at: DateTime<Utc>) -> Result<()>;

    /// Ends the work begun on this computer, returning when it began, if it had.
    async fn end_work(&self, provider: &str, task_id: &TaskId) -> Result<Option<DateTime<Utc>>>;

    /// Keeps a finished stretch of work, shared with other computers through sync.
    async fn log_work(&self, session: WorkSession) -> Result<()>;

    /// Finished stretches of work that overlap `from`..`to`, for seeing where time went.
    async fn sessions(&self, provider: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<WorkSession>>;

    /// Seconds of finished work per task, across every computer, that the task list didn't count itself.
    async fn worked(&self, provider: &str) -> Result<std::collections::HashMap<TaskId, i64>>;

    /// Sets or clears a task's where-I-left-off note.
    async fn set_left_off(&self, provider: &str, task_id: &TaskId, note: Option<LeftOff>) -> Result<()>;

    /// Every task's where-I-left-off note.
    async fn left_off(&self, provider: &str) -> Result<std::collections::HashMap<TaskId, LeftOff>>;

    /// Every task's checklist, steps in the order they were added.
    async fn checklists(&self, provider: &str) -> Result<std::collections::HashMap<TaskId, Vec<CheckItem>>>;

    async fn add_check(&self, provider: &str, task_id: &TaskId, item: CheckItem) -> Result<()>;

    async fn set_check(&self, id: &str, done: bool) -> Result<()>;

    async fn remove_check(&self, id: &str) -> Result<()>;

    /// Drops everything kept about a task, for when the task itself is deleted.
    async fn forget(&self, provider: &str, task_id: &TaskId) -> Result<()>;
}
