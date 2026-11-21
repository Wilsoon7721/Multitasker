use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use multitasker_activity::idle_time;
use multitasker_core::{TaskStatus, TaskView};
use tauri::{AppHandle, Manager};

use crate::commands::{self, task_views};
use crate::detection::Paused;
use crate::{settings, AppState};

/// How often the time away is checked.
const CHECK_EVERY: StdDuration = StdDuration::from_secs(20);
/// When someone last touched this computer, kept so time asleep or with MultiTasker closed counts as away.
const LAST_HERE: &str = "last-here.txt";

/// Since when nobody has been at this computer, if for at least `limit`; a jump past `last_here` means it slept or MultiTasker was closed.
fn away_since(now: DateTime<Utc>, idle: Duration, last_here: Option<DateTime<Utc>>, limit: Duration) -> Option<DateTime<Utc>> {
    let present = now - idle;
    if idle >= limit {
        return Some(present);
    }
    last_here.filter(|h| present - *h >= limit)
}

/// Tasks in progress on this computer that were started before the person left.
fn to_pause<'a>(tasks: &'a [TaskView], device: &str, since: DateTime<Utc>) -> Vec<&'a TaskView> {
    tasks
        .iter()
        .filter(|t| t.task.status == TaskStatus::Todo && t.in_progress)
        .filter(|t| t.in_progress_on.iter().any(|d| d == device))
        .filter(|t| t.started_at.is_none_or(|at| at < since))
        .collect()
}

fn read_last_here(path: &Path) -> Option<DateTime<Utc>> {
    let text = std::fs::read_to_string(path).ok()?;
    DateTime::parse_from_rfc3339(text.trim()).ok().map(|t| t.with_timezone(&Utc))
}

/// Pauses this computer's tasks once nobody has been at it for a while, for as long as the app runs.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("away".into())
        .spawn(move || {
            let file: PathBuf = app.state::<AppState>().data_dir.join(LAST_HERE);
            let mut last_here = read_last_here(&file);
            loop {
                check(&app, &file, &mut last_here);
                std::thread::sleep(CHECK_EVERY);
            }
        })
        .expect("failed to start the away thread");
}

fn check(app: &AppHandle, file: &Path, last_here: &mut Option<DateTime<Utc>>) {
    let state = app.state::<AppState>();
    let now = Utc::now();
    let idle = Duration::from_std(idle_time()).unwrap_or_default();
    let limit = settings::load(&state.data_dir).pause_after_away.filter(|m| *m > 0);

    if let Some(since) = limit.and_then(|m| away_since(now, idle, *last_here, Duration::minutes(m.into()))) {
        pause(app, &state, since);
    }

    let present = now - idle;
    if last_here.is_none_or(|h| present > h) {
        *last_here = Some(present);
        let _ = std::fs::write(file, present.to_rfc3339());
    }
}

fn pause(app: &AppHandle, state: &AppState, since: DateTime<Utc>) {
    let Ok(tasks) = tauri::async_runtime::block_on(task_views(state)) else { return };
    let mut paused = Vec::new();
    for t in to_pause(&tasks, &state.device.id, since) {
        let stopped = tauri::async_runtime::block_on(commands::stop_task(app, state, &t.task.id, since));
        match stopped {
            Ok(()) => paused.push(Paused { task_id: t.task.id.clone(), title: t.task.title.clone(), since }),
            Err(e) => eprintln!("couldn't pause {} while away: {e}", t.task.id),
        }
    }
    if !paused.is_empty() {
        state.detector.add_paused(app, paused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitasker_core::Task;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-21T09:00:00Z").unwrap().with_timezone(&Utc) + Duration::minutes(minutes)
    }

    #[test]
    fn notices_time_away() {
        let limit = Duration::minutes(15);
        assert_eq!(away_since(at(30), Duration::minutes(5), Some(at(24)), limit), None, "still here");
        assert_eq!(away_since(at(30), Duration::minutes(20), Some(at(9)), limit), Some(at(10)), "idle for a while");
        // Back from sleep or a night with MultiTasker closed: input is fresh, but the last sign of life is old.
        assert_eq!(away_since(at(600), Duration::seconds(3), Some(at(10)), limit), Some(at(10)));
        assert_eq!(away_since(at(30), Duration::zero(), None, limit), None, "first run knows nothing yet");
    }

    #[test]
    fn pauses_only_this_computers_older_tasks() {
        let view = |id: &str, device: &str, started: i64| TaskView {
            task: Task {
                id: id.into(),
                title: id.into(),
                description: None,
                category: None,
                due_date: None,
                due_at: None,
                status: TaskStatus::Todo,
                created_at: at(0),
                updated_at: at(0),
                completed_at: None,
            },
            in_progress: true,
            started_at: Some(at(started)),
            in_progress_on: vec![device.into()],
            started_on_name: None,
            attachments: Vec::new(),
            deliverables: 0,
            worked_secs: 0,
        };
        let tasks = vec![view("old", "me", 0), view("elsewhere", "laptop", 0), view("after", "me", 20)];
        let ids: Vec<_> = to_pause(&tasks, "me", at(10)).into_iter().map(|t| t.task.id.clone()).collect();
        assert_eq!(ids, vec!["old"]);
    }
}
