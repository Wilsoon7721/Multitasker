use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{Attachment, DeviceId, NewTask, Result, Task, TaskChanges, TaskId};

/// A source of tasks; MultiTasker keeps its extras in a [`crate::StateStore`] unless [`TaskProvider::extras`] says otherwise.
#[async_trait]
pub trait TaskProvider: Send + Sync {
    /// A stable, unique key for this provider (e.g. `"sqlite"`), used to namespace its task ids in the state store.
    fn key(&self) -> &str;

    /// A human-readable name for settings and the UI.
    fn display_name(&self) -> &str;

    /// Whether tasks can have a due time as well as a date.
    fn supports_due_time(&self) -> bool {
        true
    }

    /// Where the task list keeps MultiTasker's extras itself; `None` means MultiTasker keeps (and syncs) them.
    fn extras(&self) -> Option<&dyn ExtrasStore> {
        None
    }

    async fn list(&self) -> Result<Vec<Task>>;

    async fn create(&self, task: NewTask) -> Result<Task>;

    async fn update(&self, id: &TaskId, changes: TaskChanges) -> Result<Task>;

    async fn delete(&self, id: &TaskId) -> Result<()>;
}

/// MultiTasker's extras for one task, as a task list that keeps them reports them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskExtras {
    pub started_at: Option<DateTime<Utc>>,
    /// The computer it was started on.
    pub started_on: Option<DeviceId>,
    pub started_on_name: Option<String>,
    pub attachments: Vec<Attachment>,
}

/// A task list that holds in-progress marks and attachments itself, so they need no separate sync.
#[async_trait]
pub trait ExtrasStore: Send + Sync {
    /// Extras for every task that has any.
    async fn extras(&self) -> Result<HashMap<TaskId, TaskExtras>>;

    /// Marks a task in progress; starting again keeps when and where it was first started.
    async fn start(&self, id: &TaskId, device: &DeviceId, device_name: &str) -> Result<()>;

    async fn stop(&self, id: &TaskId) -> Result<()>;

    /// Stores an attachment built with [`crate::Attachment::new`], returning it as stored (with the list's own id).
    async fn add_attachment(&self, attachment: Attachment) -> Result<Attachment>;

    async fn remove_attachment(&self, id: &str) -> Result<()>;
}
