use std::collections::HashSet;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use multitasker_core::{TaskStatus, TaskView};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::commands::task_views;
use crate::settings::{self, Reminders};
use crate::AppState;

/// How often due times are checked.
const CHECK_EVERY: StdDuration = StdDuration::from_secs(20);
/// A reminder this late is skipped, so starting the app doesn't replay a backlog of old ones.
const GRACE: Duration = Duration::minutes(2);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Sent {
    task_id: String,
    early: bool,
    due_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
struct Reminder {
    key: Sent,
    title: String,
    body: String,
}

/// The reminders that should go out now and haven't yet.
fn due_now(tasks: &[TaskView], prefs: &Reminders, now: DateTime<Utc>, sent: &HashSet<Sent>) -> Vec<Reminder> {
    let mut out = Vec::new();
    for t in tasks.iter().filter(|t| t.task.status == TaskStatus::Todo) {
        let Some(due_at) = t.task.due_at else { continue };
        let mut moments = Vec::new();
        if let Some(minutes) = prefs.minutes_before.filter(|m| *m > 0) {
            moments.push((true, due_at - Duration::minutes(minutes.into()), format!("Due in {minutes} minutes")));
        }
        if prefs.at_due_time {
            moments.push((false, due_at, "Due now".to_owned()));
        }
        for (early, at, title) in moments {
            let key = Sent { task_id: t.task.id.clone(), early, due_at };
            if now >= at && now - at < GRACE && !sent.contains(&key) {
                out.push(Reminder { key, title, body: t.task.title.clone() });
            }
        }
    }
    out
}

/// Checks due times in the background for as long as the app runs.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("reminders".into())
        .spawn(move || {
            let mut sent: HashSet<Sent> = HashSet::new();
            loop {
                let state = app.state::<AppState>();
                let prefs = settings::load(&state.data_dir).reminders;
                if let Ok(tasks) = tauri::async_runtime::block_on(task_views(&state)) {
                    for reminder in due_now(&tasks, &prefs, Utc::now(), &sent) {
                        let _ = app.notification().builder().title(&reminder.title).body(&reminder.body).show();
                        sent.insert(reminder.key);
                    }
                }
                std::thread::sleep(CHECK_EVERY);
            }
        })
        .expect("failed to start the reminders thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitasker_core::Task;

    fn task(id: &str, due_at: Option<DateTime<Utc>>, status: TaskStatus) -> TaskView {
        let now = Utc::now();
        TaskView {
            task: Task {
                id: id.into(),
                title: format!("Task {id}"),
                description: None,
                category: None,
                due_date: due_at.map(|d| d.date_naive()),
                due_at,
                status,
                created_at: now,
                updated_at: now,
                completed_at: None,
            },
            in_progress: false,
            started_at: None,
            in_progress_on: Vec::new(),
            attachments: Vec::new(),
        }
    }

    const PREFS: Reminders = Reminders { minutes_before: Some(15), at_due_time: true };

    #[test]
    fn reminds_before_and_at_the_due_time_once_each() {
        let now = Utc::now();
        let tasks = [task("a", Some(now + Duration::minutes(15)), TaskStatus::Todo)];
        let mut sent = HashSet::new();

        let early = due_now(&tasks, &PREFS, now, &sent);
        assert_eq!(early.len(), 1);
        assert_eq!((early[0].title.as_str(), early[0].body.as_str()), ("Due in 15 minutes", "Task a"));
        sent.insert(early[0].key.clone());
        assert!(due_now(&tasks, &PREFS, now + Duration::seconds(20), &sent).is_empty(), "sent once only");

        let at_due = due_now(&tasks, &PREFS, now + Duration::minutes(15), &sent);
        assert_eq!(at_due.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Due now"]);
    }

    #[test]
    fn skips_done_untimed_and_long_past_tasks() {
        let now = Utc::now();
        let tasks = [
            task("done", Some(now), TaskStatus::Done),
            task("untimed", None, TaskStatus::Todo),
            task("stale", Some(now - Duration::hours(1)), TaskStatus::Todo),
        ];
        assert!(due_now(&tasks, &PREFS, now, &HashSet::new()).is_empty());
    }

    #[test]
    fn a_moved_due_time_reminds_again() {
        let now = Utc::now();
        let mut sent = HashSet::new();
        let first = due_now(&[task("a", Some(now), TaskStatus::Todo)], &PREFS, now, &sent);
        sent.extend(first.into_iter().map(|r| r.key));
        let later = now + Duration::minutes(30);
        assert_eq!(due_now(&[task("a", Some(later), TaskStatus::Todo)], &PREFS, later, &sent).len(), 1);
    }

    #[test]
    fn follows_the_settings() {
        let now = Utc::now();
        let tasks = [task("a", Some(now), TaskStatus::Todo)];
        let off = Reminders { minutes_before: None, at_due_time: false };
        assert!(due_now(&tasks, &off, now, &HashSet::new()).is_empty());
    }
}
