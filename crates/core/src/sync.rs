use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::{Attachment, DeviceId, TaskId, TaskStatus};

/// When a change happened, as a hybrid logical clock; string order is time order across every device.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Stamp(String);

impl Stamp {
    pub fn new(millis: i64, counter: u32, device: &str) -> Self {
        Stamp(format!("{millis:016}-{counter:06}-{device}"))
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(3, '-');
        let (millis, counter, device) = (parts.next()?, parts.next()?, parts.next()?);
        Some(Stamp::new(millis.parse().ok()?, counter.parse().ok()?, device))
    }

    pub fn millis(&self) -> i64 {
        self.0[..16].parse().unwrap_or_default()
    }

    pub fn counter(&self) -> u32 {
        self.0[17..23].parse().unwrap_or_default()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The next stamp for a local change: wall-clock time, but never behind the last stamp seen.
    pub fn next(last: Option<&Stamp>, now_millis: i64, device: &str) -> Stamp {
        match last {
            Some(last) if last.millis() >= now_millis => Stamp::new(last.millis(), last.counter() + 1, device),
            _ => Stamp::new(now_millis, 0, device),
        }
    }
}

/// Task fields a change sets; absent fields are left alone, and `Some(None)` clears one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::task::double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::task::double_option")]
    pub category: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::task::double_option")]
    pub due_date: Option<Option<NaiveDate>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::task::double_option")]
    pub due_at: Option<Option<DateTime<Utc>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::task::double_option")]
    pub completed_at: Option<Option<DateTime<Utc>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

/// One change to replicate to other devices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Change {
    Task { id: TaskId, fields: TaskFields },
    TaskDeleted { id: TaskId },
    /// A task started (`started_at`) or stopped (`None`) on a device.
    Focus { provider: String, task_id: TaskId, device: DeviceId, started_at: Option<DateTime<Utc>> },
    AttachmentAdded { attachment: Attachment },
    AttachmentRemoved { id: String },
    /// A finished stretch of work; never changed once made.
    WorkLogged { session: crate::WorkSession },
    /// A task's where-I-left-off note was set or cleared (`None`).
    LeftOff { provider: String, task_id: TaskId, note: Option<crate::LeftOff> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub stamp: Stamp,
    #[serde(flatten)]
    pub change: Change,
}

/// A computer taking part in sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub last_seen: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_order_by_time_then_counter_then_device() {
        let a = Stamp::new(1_000, 0, "pc-b");
        let b = Stamp::new(1_000, 1, "pc-a");
        let c = Stamp::new(2_000, 0, "pc-a");
        assert!(a < b && b < c);
        assert_eq!(Stamp::parse(b.as_str()), Some(b.clone()));
        assert_eq!((c.millis(), c.counter()), (2_000, 0));
    }

    #[test]
    fn next_stamps_never_go_backwards() {
        let last = Stamp::new(5_000, 3, "pc-a");
        assert_eq!(Stamp::next(Some(&last), 4_000, "pc-a"), Stamp::new(5_000, 4, "pc-a"));
        assert_eq!(Stamp::next(Some(&last), 6_000, "pc-a"), Stamp::new(6_000, 0, "pc-a"));
        assert_eq!(Stamp::next(None, 7_000, "pc-a"), Stamp::new(7_000, 0, "pc-a"));
    }

    #[test]
    fn ops_round_trip_through_json_keeping_cleared_fields() {
        let op = Op {
            stamp: Stamp::new(1, 0, "pc"),
            change: Change::Task {
                id: "t".into(),
                fields: TaskFields { title: Some("Essay".into()), category: Some(None), ..Default::default() },
            },
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""category":null"#) && !json.contains("due_date"), "{json}");
        assert_eq!(serde_json::from_str::<Op>(&json).unwrap(), op);
    }
}
