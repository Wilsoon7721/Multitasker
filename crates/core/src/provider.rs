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

    /// Where the task list keeps handed-in work itself; `None` means MultiTasker keeps (and syncs) it.
    fn work(&self) -> Option<&dyn WorkStore> {
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
    /// Seconds worked on it before the stretch in progress now, across computers.
    pub worked_secs: i64,
    pub left_off: Option<crate::LeftOff>,
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

    /// Adds a finished stretch of work to the task's time spent.
    async fn add_worked(&self, id: &TaskId, secs: i64) -> Result<()>;

    /// Sets or clears the task's where-I-left-off note.
    async fn set_left_off(&self, id: &TaskId, note: Option<crate::LeftOff>) -> Result<()>;
}

/// A copy of handed-in work, as a task list that keeps it reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct HandedInFile {
    /// The list's own id for it (a comment, a file block).
    pub id: String,
    pub task_id: TaskId,
    pub name: String,
    pub sha256: String,
    pub size: u64,
    pub device: Option<DeviceId>,
    pub added_at: DateTime<Utc>,
}

/// A task list that holds handed-in files itself, so every computer signed in to it gets them.
#[async_trait]
pub trait WorkStore: Send + Sync {
    /// What's been handed in for one task.
    async fn handed_in(&self, task_id: &TaskId) -> Result<Vec<HandedInFile>>;

    /// What's been handed in for every task, for catching up in the background.
    async fn all_handed_in(&self) -> Result<Vec<HandedInFile>>;

    /// Uploads a file; `file` says what it is, and its id is replaced with the list's own.
    async fn hand_in(&self, file: HandedInFile, bytes: Vec<u8>, device_name: &str) -> Result<HandedInFile>;

    async fn remove_handed_in(&self, id: &str) -> Result<()>;

    /// The file's content.
    async fn download(&self, file: &HandedInFile) -> Result<Vec<u8>>;
}
