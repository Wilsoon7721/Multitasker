use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A provider's own identifier for a task, kept as an opaque string so any backend's ids fit.
pub type TaskId = String;

pub const MAX_TITLE: usize = 200;
pub const MAX_CATEGORY: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    Done,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Todo => "todo",
            TaskStatus::Done => "done",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "todo" => Ok(TaskStatus::Todo),
            "done" => Ok(TaskStatus::Done),
            other => Err(Error::Invalid(format!("unknown task status '{other}'"))),
        }
    }
}

/// The shared task model: only what every task list can represent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub due_date: Option<NaiveDate>,
    /// The exact moment it's due, when a time was given; `due_date` is that moment's date where it was set.
    pub due_at: Option<DateTime<Utc>>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// What it takes to create a task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewTask {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub due_date: Option<NaiveDate>,
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
}

/// A partial edit. `None` leaves a field alone; `Some(None)` clears an optional one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskChanges {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    pub category: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    pub due_date: Option<Option<NaiveDate>>,
    #[serde(default, with = "double_option")]
    pub due_at: Option<Option<DateTime<Utc>>>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
}

/// A task as the UI shows it: the provider's task plus MultiTasker's own state for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    pub in_progress: bool,
    pub started_at: Option<DateTime<Utc>>,
    /// The name of the computer it was started on, when the task list keeps it (MultiTasker's own state keeps ids).
    pub started_on_name: Option<String>,
    /// The devices it's in progress on, so other computers can show where.
    pub in_progress_on: Vec<crate::DeviceId>,
    pub attachments: Vec<crate::Attachment>,
    /// How many pieces of work have been handed in for it.
    pub deliverables: u32,
    /// Seconds worked on it before the stretch in progress now, across computers.
    pub worked_secs: i64,
    /// Where work on it stopped, as noted when it was paused.
    pub left_off: Option<crate::LeftOff>,
}

/// Trims text and turns blank into `None`.
pub fn clean(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

pub fn check_title(title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Invalid("A task needs a title.".into()));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(Error::Invalid(format!("Keep the title under {MAX_TITLE} characters.")));
    }
    Ok(title.to_owned())
}

pub fn check_category(category: Option<String>) -> Result<Option<String>> {
    let category = clean(category).map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "));
    if category.as_ref().is_some_and(|c| c.chars().count() > MAX_CATEGORY) {
        return Err(Error::Invalid(format!("Keep the category under {MAX_CATEGORY} characters.")));
    }
    Ok(category)
}

/// Lets a JSON field tell "absent" (leave alone) from `null` (clear).
pub(crate) mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(value: &Option<Option<T>>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_tell_absent_from_null() {
        let absent: TaskChanges = serde_json::from_str(r#"{}"#).unwrap();
        assert!(absent.category.is_none());

        let cleared: TaskChanges = serde_json::from_str(r#"{"category":null}"#).unwrap();
        assert_eq!(cleared.category, Some(None));

        let set: TaskChanges = serde_json::from_str(r#"{"due_date":"2026-12-01"}"#).unwrap();
        assert_eq!(set.due_date, Some(NaiveDate::from_ymd_opt(2026, 12, 1)));
    }

    #[test]
    fn titles_and_categories_are_checked() {
        assert!(check_title("   ").is_err());
        assert_eq!(check_title("  Revise GS notes ").unwrap(), "Revise GS notes");
        assert_eq!(check_category(Some("  school   work ".into())).unwrap(), Some("school work".into()));
        assert_eq!(check_category(Some("   ".into())).unwrap(), None);
    }
}
