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

/// How long the computer can be off, asleep or without MultiTasker before its tasks are paused, when pausing while idle is off.
const OFF_LIMIT_MINUTES: u32 = 15;

/// Since when nobody has been at this computer: idle for `idle_limit`, or a jump of `off_limit` past `last_here` (asleep or closed).
fn away_since(
    now: DateTime<Utc>,
    idle: Duration,
    last_here: Option<DateTime<Utc>>,
    idle_limit: Option<Duration>,
    off_limit: Duration,
) -> Option<DateTime<Utc>> {
    let present = now - idle;
    if idle_limit.is_some_and(|limit| idle >= limit) {
        return Some(present);
    }
    last_here.filter(|h| present - *h >= off_limit)
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
    let setting = settings::load(&state.data_dir).pause_after_away.filter(|m| *m > 0);
    let idle_limit = setting.map(|m| Duration::minutes(m.into()));
    // Time with the computer off is never work, so it's noticed even when pausing while idle is turned off.
    let off_limit = Duration::minutes(setting.unwrap_or(OFF_LIMIT_MINUTES).into());

    if let Some(since) = away_since(now, idle, *last_here, idle_limit, off_limit) {
        // Right after waking up the network may not be back. Until the tasks are paused, the last sign of life stays
        // put, so the time away is still noticed on the next check instead of being counted as work.
        if !pause(app, &state, since) {
            return;
        }
    }

    let present = now - idle;
    if last_here.is_none_or(|h| present > h) {
        *last_here = Some(present);
        let _ = std::fs::write(file, present.to_rfc3339());
    }
}

/// Pauses this computer's tasks as of when the person left; returns whether every one that needed it was paused.
fn pause(app: &AppHandle, state: &AppState, since: DateTime<Utc>) -> bool {
    let tasks = match tauri::async_runtime::block_on(task_views(state)) {
        Ok(tasks) => tasks,
        Err(e) => {
            eprintln!("couldn't check what to pause while away, trying again shortly: {e}");
            return false;
        }
    };
    let mut paused = Vec::new();
    let mut all = true;
    for t in to_pause(&tasks, &state.device.id, since) {
        let stopped = tauri::async_runtime::block_on(commands::stop_task(app, state, &t.task.id, since));
        match stopped {
            Ok(()) => paused.push(Paused { task_id: t.task.id.clone(), title: t.task.title.clone(), since }),
            Err(e) => {
                eprintln!("couldn't pause {} while away, trying again shortly: {e}", t.task.id);
                all = false;
            }
        }
    }
    if !paused.is_empty() {
        state.detector.add_paused(app, paused);
    }
    all
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
        let on = Some(limit);
        assert_eq!(away_since(at(30), Duration::minutes(5), Some(at(24)), on, limit), None, "still here");
        assert_eq!(away_since(at(30), Duration::minutes(20), Some(at(9)), on, limit), Some(at(10)), "idle for a while");
        // Back from sleep or a night with MultiTasker closed: input is fresh, but the last sign of life is old.
        assert_eq!(away_since(at(600), Duration::seconds(3), Some(at(10)), on, limit), Some(at(10)));
        assert_eq!(away_since(at(30), Duration::zero(), None, on, limit), None, "first run knows nothing yet");
    }

    #[test]
    fn notices_time_off_even_when_idle_pausing_is_off() {
        let limit = Duration::minutes(15);
        assert_eq!(away_since(at(30), Duration::minutes(20), Some(at(9)), None, limit), None, "idle doesn't count");
        assert_eq!(away_since(at(600), Duration::seconds(3), Some(at(10)), None, limit), Some(at(10)), "a night off");
        assert_eq!(away_since(at(12), Duration::seconds(3), Some(at(10)), None, limit), None, "a quick restart");
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
            left_off: None,
            checklist: Vec::new(),
        };
        let tasks = vec![view("old", "me", 0), view("elsewhere", "laptop", 0), view("after", "me", 20)];
        let ids: Vec<_> = to_pause(&tasks, "me", at(10)).into_iter().map(|t| t.task.id.clone()).collect();
        assert_eq!(ids, vec!["old"]);
    }
}
