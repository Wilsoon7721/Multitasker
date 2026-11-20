use async_trait::async_trait;

use crate::{NewTask, Result, Task, TaskChanges, TaskId};

/// A source of tasks; MultiTasker's own extras (in-progress state, links) live in a [`crate::StateStore`] instead.
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

    async fn list(&self) -> Result<Vec<Task>>;

    async fn create(&self, task: NewTask) -> Result<Task>;

    async fn update(&self, id: &TaskId, changes: TaskChanges) -> Result<Task>;

    async fn delete(&self, id: &TaskId) -> Result<()>;
}
