use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use multitasker_activity::{foreground_window, idle_time, matches, FileWatcher, Observation, Strength};
use multitasker_core::{Attachment, AttachmentKind, TaskId, TaskStatus, TaskView};
use serde::Serialize;
use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, Listener, Manager, Wry};

use crate::commands::{task_views, TASKS_CHANGED};
use crate::AppState;

/// Tells every window what's being worked on, whenever that changes.
pub const ACTIVITY: &str = "activity";

/// How often the window in front is checked.
const POLL: Duration = Duration::from_millis(1500);
/// How often the task list is re-read even without a change event, to pick up files that appeared.
const REFRESH: Duration = Duration::from_secs(30);
/// No keyboard or mouse input for this long means nobody is working on anything.
const IDLE: Duration = Duration::from_secs(5 * 60);
/// A dismissed suggestion stays quiet for this long.
const DISMISS_FOR: Duration = Duration::from_secs(30 * 60);
/// A suggestion disappears once its match hasn't been seen for this long.
const SUGGESTION_TTL: Duration = Duration::from_secs(5 * 60);
/// Activity older than this is dropped from snapshots.
const FORGET_AFTER: Duration = Duration::from_secs(60 * 60);
/// Re-announce ongoing activity this often, so "active now" stays fresh in the UI.
const TOUCH_EVERY: Duration = Duration::from_secs(30);
/// Browsers whose tab in front the extension can report.
const BROWSERS: &[&str] = &["chrome.exe", "msedge.exe", "brave.exe", "vivaldi.exe", "opera.exe", "arc.exe"];

/// An in-progress task that was just worked on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Activity {
    pub task_id: TaskId,
    pub at: DateTime<Utc>,
    pub reason: String,
}

/// A task that isn't in progress but looks like it's being worked on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub task_id: TaskId,
    pub title: String,
    pub reason: String,
}

/// A task paused because nobody was at the computer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Paused {
    pub task_id: TaskId,
    pub title: String,
    /// When the person left.
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub enabled: bool,
    pub active: Vec<Activity>,
    pub suggestion: Option<Suggestion>,
    pub paused: Vec<Paused>,
}

/// The tray's "Detect what I'm working on" item, kept so the UI can tick or untick it.
pub struct TrayToggle(pub CheckMenuItem<Wry>);

#[derive(Default)]
struct Inner {
    tasks: Vec<TaskView>,
    active: HashMap<TaskId, Activity>,
    suggestion: Option<(Suggestion, Instant)>,
    dismissed: HashMap<TaskId, Instant>,
    paused: Vec<Paused>,
}

pub struct Detector {
    enabled: AtomicBool,
    dirty: AtomicBool,
    inner: Mutex<Inner>,
    /// The browser tab in front, as the extension last reported it; `None` once the browser lost focus.
    tab: Mutex<Option<String>>,
}

impl Detector {
    pub fn new(enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            dirty: AtomicBool::new(true),
            inner: Mutex::default(),
            tab: Mutex::new(None),
        })
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Notes the browser tab in front, and returns the open tasks it's linked to, best match first.
    pub fn set_tab(&self, url: Option<String>) -> Vec<(TaskId, String)> {
        *self.tab.lock().unwrap_or_else(|e| e.into_inner()) = url.clone();
        let Some(url) = url else { return Vec::new() };
        let inner = self.inner();
        let open: Vec<&TaskView> = inner.tasks.iter().filter(|t| t.task.status == TaskStatus::Todo).collect();
        let attachments: Vec<Attachment> = open.iter().flat_map(|t| t.attachments.iter().cloned()).collect();
        matches(&attachments, &Observation::Tab { url })
            .into_iter()
            .filter_map(|m| open.iter().find(|t| t.task.id == m.task_id).map(|t| (m.task_id, t.task.title.clone())))
            .collect()
    }

    fn tab(&self) -> Option<String> {
        self.tab.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> Snapshot {
        let inner = self.inner();
        Snapshot {
            enabled: self.enabled(),
            active: inner.active.values().cloned().collect(),
            suggestion: inner.suggestion.as_ref().map(|(s, _)| s.clone()),
            paused: inner.paused.clone(),
        }
    }

    fn emit(&self, app: &AppHandle) {
        let _ = app.emit(ACTIVITY, self.snapshot());
    }

    pub fn set_enabled(&self, app: &AppHandle, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            let mut inner = self.inner();
            inner.active.clear();
            inner.suggestion = None;
        }
        self.emit(app);
    }

    pub fn dismiss(&self, app: &AppHandle, task_id: &TaskId) {
        {
            let mut inner = self.inner();
            inner.dismissed.insert(task_id.clone(), Instant::now());
            if inner.suggestion.as_ref().is_some_and(|(s, _)| &s.task_id == task_id) {
                inner.suggestion = None;
            }
        }
        self.emit(app);
    }

    /// Notes tasks that were paused while nobody was here, so the widget can offer to resume them.
    pub fn add_paused(&self, app: &AppHandle, paused: Vec<Paused>) {
        {
            let mut inner = self.inner();
            inner.paused.retain(|p| !paused.iter().any(|n| n.task_id == p.task_id));
            inner.paused.extend(paused);
        }
        self.emit(app);
    }

    pub fn dismiss_paused(&self, app: &AppHandle) {
        self.inner().paused.clear();
        self.emit(app);
    }

    /// Takes the latest task list, dropping activity and suggestions that no longer apply.
    fn refresh(&self, tasks: Vec<TaskView>) -> bool {
        let mut inner = self.inner();
        let in_progress: HashSet<&TaskId> = tasks.iter().filter(|t| t.in_progress).map(|t| &t.task.id).collect();
        let before = inner.active.len();
        inner.active.retain(|id, _| in_progress.contains(id));
        let mut changed = inner.active.len() != before;

        let still_valid = |s: &Suggestion| {
            tasks.iter().any(|t| t.task.id == s.task_id && t.task.status == TaskStatus::Todo && !t.in_progress)
        };
        if inner.suggestion.as_ref().is_some_and(|(s, _)| !still_valid(s)) {
            inner.suggestion = None;
            changed = true;
        }
        // A paused task that was started again, finished or deleted needs no reminder.
        let before = inner.paused.len();
        inner.paused.retain(|p| tasks.iter().any(|t| t.task.id == p.task_id && t.task.status == TaskStatus::Todo && !t.in_progress));
        changed |= inner.paused.len() != before;
        inner.tasks = tasks;
        changed
    }

    /// Records what an observation says about the open tasks; returns whether anything visible changed.
    fn observe(&self, observation: &Observation) -> bool {
        let mut inner = self.inner();
        let open: Vec<&TaskView> = inner.tasks.iter().filter(|t| t.task.status == TaskStatus::Todo).collect();
        let attachments: Vec<Attachment> = open.iter().flat_map(|t| t.attachments.iter().cloned()).collect();
        let found = matches(&attachments, observation);
        let Some(top) = found.first().map(|m| m.strength) else { return false };

        let best: Vec<_> = found.into_iter().filter(|m| m.strength == top).collect();
        let in_progress: HashSet<TaskId> = open.iter().filter(|t| t.in_progress).map(|t| t.task.id.clone()).collect();
        let titles: HashMap<TaskId, String> = open.iter().map(|t| (t.task.id.clone(), t.task.title.clone())).collect();
        let now = Utc::now();
        let mut changed = false;

        let mut any_active = false;
        for m in best.iter().filter(|m| in_progress.contains(&m.task_id)) {
            any_active = true;
            let fresh = Activity { task_id: m.task_id.clone(), at: now, reason: m.reason.clone() };
            let stale = match inner.active.get(&m.task_id) {
                Some(a) => a.reason != fresh.reason || (now - a.at).to_std().unwrap_or_default() >= TOUCH_EVERY,
                None => true,
            };
            if stale {
                inner.active.insert(m.task_id.clone(), fresh);
                changed = true;
            }
        }

        // Only suggest when one task clearly wins and nothing already in progress explains the activity.
        if !any_active && top >= Strength::Medium && best.len() == 1 {
            let m = &best[0];
            let dismissed = inner.dismissed.get(&m.task_id).is_some_and(|at| at.elapsed() < DISMISS_FOR);
            if !dismissed {
                let title = titles.get(&m.task_id).cloned().unwrap_or_default();
                let suggestion = Suggestion { task_id: m.task_id.clone(), title, reason: m.reason.clone() };
                changed |= inner.suggestion.as_ref().map(|(s, _)| s) != Some(&suggestion);
                inner.suggestion = Some((suggestion, Instant::now()));
            }
        }
        changed
    }

    /// Lets old suggestions and activity lapse.
    fn expire(&self) -> bool {
        let mut inner = self.inner();
        let mut changed = false;
        if inner.suggestion.as_ref().is_some_and(|(_, seen)| seen.elapsed() >= SUGGESTION_TTL) {
            inner.suggestion = None;
            changed = true;
        }
        let now = Utc::now();
        let before = inner.active.len();
        inner.active.retain(|_, a| (now - a.at).to_std().unwrap_or_default() < FORGET_AFTER);
        inner.dismissed.retain(|_, at| at.elapsed() < DISMISS_FOR);
        changed || inner.active.len() != before
    }
}

/// Starts watching in the background for as long as the app runs.
pub fn start(app: &AppHandle, detector: Arc<Detector>) {
    let dirty = detector.clone();
    app.listen_any(TASKS_CHANGED, move |_| dirty.dirty.store(true, Ordering::Relaxed));

    let app = app.clone();
    std::thread::Builder::new()
        .name("activity".into())
        .spawn(move || watch(app, detector))
        .expect("failed to start the activity thread");
}

fn watch(app: AppHandle, d: Arc<Detector>) {
    let (tx, rx) = mpsc::channel::<PathBuf>();
    let mut watcher = FileWatcher::new(move |path| {
        let _ = tx.send(path);
    })
    .ok();
    let me = std::process::id();
    let mut last_refresh: Option<Instant> = None;
    let mut last_poll = Instant::now();

    loop {
        if d.dirty.swap(false, Ordering::Relaxed) || last_refresh.is_none_or(|t| t.elapsed() >= REFRESH) {
            let state = app.state::<AppState>();
            if let Ok(tasks) = tauri::async_runtime::block_on(task_views(&state)) {
                if let Some(w) = watcher.as_mut() {
                    w.watch_only(watch_paths(&tasks, &state.device.id));
                }
                if d.refresh(tasks) {
                    d.emit(&app);
                }
            }
            last_refresh = Some(Instant::now());
        }

        let wait = POLL.saturating_sub(last_poll.elapsed());
        match rx.recv_timeout(wait) {
            Ok(path) => {
                if d.enabled() && d.observe(&Observation::FileSaved { path: path.display().to_string() }) {
                    d.emit(&app);
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            // No file watcher (it failed to start): just pace the window checks.
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(wait),
        }

        last_poll = Instant::now();
        let mut changed = d.expire();
        if d.enabled() && idle_time() < IDLE {
            if let Some(w) = foreground_window().filter(|w| w.pid != me) {
                // With a browser in front, the tab the extension reported says more than the window title.
                let tab = BROWSERS.iter().any(|b| w.exe.eq_ignore_ascii_case(b)).then(|| d.tab()).flatten();
                changed |= d.observe(&Observation::Window { exe: w.exe, title: w.title });
                if let Some(url) = tab {
                    changed |= d.observe(&Observation::Tab { url });
                }
            }
        }
        if changed {
            d.emit(&app);
        }
    }
}

/// Linked files and folders of open tasks that live on this computer.
fn watch_paths(tasks: &[TaskView], device: &str) -> Vec<PathBuf> {
    tasks
        .iter()
        .filter(|t| t.task.status == TaskStatus::Todo)
        .flat_map(|t| &t.attachments)
        .filter(|a| !matches!(a.kind, AttachmentKind::Link | AttachmentKind::App))
        .filter(|a| a.device.as_deref() == Some(device))
        .map(|a| PathBuf::from(&a.target))
        .collect()
}

/// Ticks or unticks the tray item after the setting changed elsewhere.
pub fn sync_tray(app: &AppHandle, enabled: bool) {
    if let Some(toggle) = app.try_state::<TrayToggle>() {
        let _ = toggle.0.set_checked(enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitasker_core::Task;

    fn view(id: &str, in_progress: bool, attachments: Vec<(AttachmentKind, &str, &str)>) -> TaskView {
        let now = Utc::now();
        TaskView {
            task: Task {
                id: id.into(),
                title: format!("Task {id}"),
                description: None,
                category: None,
                due_date: None,
                due_at: None,
                status: TaskStatus::Todo,
                created_at: now,
                updated_at: now,
                completed_at: None,
            },
            in_progress,
            started_at: in_progress.then_some(now),
            in_progress_on: if in_progress { vec!["pc".into()] } else { Vec::new() },
            started_on_name: None,
            deliverables: 0,
            attachments: attachments
                .into_iter()
                .map(|(kind, name, target)| Attachment {
                    id: format!("{id}-{name}"),
                    provider: "sqlite".into(),
                    task_id: id.into(),
                    kind,
                    name: name.into(),
                    target: target.into(),
                    device: Some("pc".into()),
                    added_at: now,
                })
                .collect(),
        }
    }

    fn word(title: &str) -> Observation {
        Observation::Window { exe: "WINWORD.EXE".into(), title: title.into() }
    }

    fn setup() -> Arc<Detector> {
        let d = Detector::new(true);
        d.refresh(vec![
            view("essay", true, vec![(AttachmentKind::App, "Word", r"C:\Office\WINWORD.EXE")]),
            view("report", false, vec![(AttachmentKind::Document, "report.docx", r"C:\Docs\report.docx")]),
        ]);
        d
    }

    #[test]
    fn marks_in_progress_tasks_active() {
        let d = setup();
        assert!(d.observe(&word("Document1 - Word")));
        let snap = d.snapshot();
        assert_eq!(snap.active.len(), 1);
        assert_eq!(snap.active[0].task_id, "essay");
        assert!(snap.suggestion.is_none());
        // Seeing the same thing again right away changes nothing visible.
        assert!(!d.observe(&word("Document1 - Word")));
    }

    #[test]
    fn suggests_a_to_do_task_whose_file_is_open() {
        let d = setup();
        assert!(d.observe(&word("report - Word")));
        let snap = d.snapshot();
        assert!(snap.active.is_empty(), "the stronger match on the other task wins");
        assert_eq!(snap.suggestion.map(|s| s.task_id), Some("report".into()));
    }

    #[test]
    fn dismissed_and_started_suggestions_go_away() {
        let d = setup();
        d.observe(&word("report - Word"));
        {
            let mut inner = d.inner();
            inner.dismissed.insert("report".into(), Instant::now());
            inner.suggestion = None;
        }
        assert!(!d.observe(&word("report - Word")));

        d.inner().dismissed.clear();
        d.observe(&word("report - Word"));
        assert!(d.refresh(vec![view("report", true, vec![])]));
        assert!(d.snapshot().suggestion.is_none());
    }
}
