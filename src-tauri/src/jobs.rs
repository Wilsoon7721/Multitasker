use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

/// Tells every window the things being waited on changed.
pub const JOBS_CHANGED: &str = "jobs-changed";

const CHECK_EVERY: Duration = Duration::from_secs(2);
/// Processor time over one check that counts as working: 15% of one core.
const BUSY: Duration = Duration::from_millis(300);
/// Below this, a program counts as resting: 3% of one core.
const QUIET: Duration = Duration::from_millis(60);
/// A program that worked and then rested this long is done.
const QUIET_FOR: Duration = Duration::from_secs(20);
/// A file that changed and then stayed the same this long is done.
const SETTLED_FOR: Duration = Duration::from_secs(15);

/// Something a task is waiting on: a long build, a render, an export, a download.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub task_id: String,
    pub task_title: String,
    /// What it is, e.g. "Blender" or "report.pdf".
    pub label: String,
    pub what: What,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// How it was seen to finish, e.g. "closed" or "went quiet".
    pub how: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum What {
    /// A program, finished when it closes, or when it and what it started have worked and then gone quiet.
    Program { pid: u32 },
    /// A file being written, finished once it has changed and then stopped changing.
    File { path: String },
    /// An AI chat writing a reply in a browser tab; the extension says when it's done.
    Tab { tab_id: i64 },
}

/// A reply this quick was probably watched as it came, so it isn't announced.
const QUICK_REPLY: Duration = Duration::from_secs(5);

/// What's been seen of a job so far, between checks.
struct Watch {
    job: Job,
    last_cpu: Option<Duration>,
    busy_seen: bool,
    quiet_since: Option<Instant>,
    last_file: Option<(u64, SystemTime)>,
    changed_seen: bool,
    settled_since: Option<Instant>,
}

static JOBS: Mutex<Vec<Watch>> = Mutex::new(Vec::new());

fn lock() -> std::sync::MutexGuard<'static, Vec<Watch>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn list() -> Vec<Job> {
    lock().iter().map(|w| w.job.clone()).collect()
}

pub fn add(app: &AppHandle, task_id: String, task_title: String, label: String, what: What) -> Job {
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        task_id,
        task_title,
        label,
        what,
        started_at: Utc::now(),
        finished_at: None,
        how: None,
    };
    let last_cpu = match &job.what {
        What::Program { pid } => multitasker_activity::tree_cpu_time(*pid),
        What::File { .. } | What::Tab { .. } => None,
    };
    let last_file = match &job.what {
        What::File { path } => file_state(path),
        What::Program { .. } | What::Tab { .. } => None,
    };
    let mut jobs = lock();
    // A new reply in the same tab replaces the last one's.
    if let What::Tab { tab_id } = &job.what {
        jobs.retain(|w| !matches!(w.job.what, What::Tab { tab_id: t } if t == *tab_id));
    }
    jobs.push(Watch {
        job: job.clone(),
        last_cpu,
        busy_seen: false,
        quiet_since: None,
        last_file,
        changed_seen: false,
        settled_since: None,
    });
    drop(jobs);
    let _ = app.emit(JOBS_CHANGED, ());
    job
}

/// An AI reply finished: announced if it took a while and its tab wasn't being looked at, otherwise let go.
pub fn tab_done(app: &AppHandle, tab_id: i64, looking: bool) {
    let finished = {
        let mut jobs = lock();
        let open = |w: &Watch| matches!(w.job.what, What::Tab { tab_id: t } if t == tab_id) && w.job.finished_at.is_none();
        let Some(i) = jobs.iter().position(open) else {
            eprintln!("AI reply in tab {tab_id} finished, but its start was never heard");
            return;
        };
        let quick = (Utc::now() - jobs[i].job.started_at).to_std().unwrap_or_default() < QUICK_REPLY;
        eprintln!("AI reply in tab {tab_id} finished: looking={looking}, quick={quick}");
        if looking || quick {
            jobs.remove(i);
            None
        } else {
            jobs[i].job.finished_at = Some(Utc::now());
            jobs[i].job.how = Some("replied".into());
            Some(jobs[i].job.clone())
        }
    };
    if let Some(job) = &finished {
        announce(app, job);
    }
    let _ = app.emit(JOBS_CHANGED, ());
}

fn announce(app: &AppHandle, job: &Job) {
    let title = match job.what {
        What::Tab { .. } => format!("{} replied", job.label),
        _ => format!("{} is done", job.label),
    };
    let body = if job.task_title.is_empty() { "Back to it?".to_owned() } else { format!("Back to “{}”?", job.task_title) };
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Goes back to what a finished job was for: its tab, if it's in a browser.
pub fn go_back(id: &str) -> Option<Job> {
    let job = lock().iter().find(|w| w.job.id == id).map(|w| w.job.clone())?;
    if let What::Tab { tab_id } = job.what {
        crate::bridge::focus_tab(tab_id);
    }
    Some(job)
}

pub fn remove(app: &AppHandle, id: &str) {
    lock().retain(|w| w.job.id != id);
    let _ = app.emit(JOBS_CHANGED, ());
}

fn file_state(path: &str) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(PathBuf::from(path)).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Looks at one job; returns how it finished, if it has.
fn check(w: &mut Watch, now: Instant) -> Option<&'static str> {
    match &w.job.what {
        What::Program { pid } => {
            let Some(cpu) = multitasker_activity::tree_cpu_time(*pid) else { return Some("closed") };
            // Processes that end take their time with them, so the total can drop; that counts as resting.
            let used = w.last_cpu.map_or(Duration::ZERO, |last| cpu.saturating_sub(last));
            w.last_cpu = Some(cpu);
            if used >= BUSY {
                w.busy_seen = true;
                w.quiet_since = None;
            } else if used < QUIET && w.busy_seen {
                let since = *w.quiet_since.get_or_insert(now);
                if now.duration_since(since) >= QUIET_FOR {
                    return Some("went quiet");
                }
            } else {
                w.quiet_since = None;
            }
            None
        }
        What::Tab { .. } => None,
        What::File { path } => {
            let state = file_state(path);
            if state != w.last_file {
                w.last_file = state;
                // A file that appears or grows is being written.
                w.changed_seen = true;
                w.settled_since = None;
                return None;
            }
            if w.changed_seen && state.is_some() {
                let since = *w.settled_since.get_or_insert(now);
                if now.duration_since(since) >= SETTLED_FOR {
                    return Some("stopped changing");
                }
            }
            None
        }
    }
}

/// Checks the things being waited on in the background, and says when one is done.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("jobs".into())
        .spawn(move || loop {
            std::thread::sleep(CHECK_EVERY);
            let now = Instant::now();
            let finished: Vec<Job> = {
                let mut jobs = lock();
                jobs.iter_mut()
                    .filter(|w| w.job.finished_at.is_none() && !matches!(w.job.what, What::Tab { .. }))
                    .filter_map(|w| {
                        let how = check(w, now)?;
                        w.job.finished_at = Some(Utc::now());
                        w.job.how = Some(how.to_owned());
                        Some(w.job.clone())
                    })
                    .collect()
            };
            for job in &finished {
                announce(&app, job);
            }
            if !finished.is_empty() {
                let _ = app.emit(JOBS_CHANGED, ());
            }
        })
        .expect("failed to start the jobs thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(what: What) -> Watch {
        Watch {
            job: Job {
                id: "j".into(),
                task_id: "t".into(),
                task_title: "Task".into(),
                label: "Thing".into(),
                what,
                started_at: Utc::now(),
                finished_at: None,
                how: None,
            },
            last_cpu: None,
            busy_seen: false,
            quiet_since: None,
            last_file: None,
            changed_seen: false,
            settled_since: None,
        }
    }

    #[test]
    fn a_closed_program_is_done() {
        let mut w = watch(What::Program { pid: u32::MAX - 3 });
        assert_eq!(check(&mut w, Instant::now()), Some("closed"));
    }

    #[test]
    fn a_running_program_that_never_worked_isnt_done() {
        let mut w = watch(What::Program { pid: std::process::id() });
        w.last_cpu = multitasker_activity::tree_cpu_time(std::process::id());
        let start = Instant::now();
        assert_eq!(check(&mut w, start), None);
        assert_eq!(check(&mut w, start + QUIET_FOR * 2), None, "resting from the start isn't finishing");
    }

    #[test]
    fn a_file_is_done_once_it_stops_changing() {
        let dir = std::env::temp_dir().join(format!("multitasker-job-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("export.mp4");
        let mut w = watch(What::File { path: path.display().to_string() });
        let start = Instant::now();
        assert_eq!(check(&mut w, start), None, "not there yet");

        std::fs::write(&path, b"part").unwrap();
        assert_eq!(check(&mut w, start), None, "appearing is being written");
        assert_eq!(check(&mut w, start + Duration::from_secs(1)), None);
        assert_eq!(check(&mut w, start + Duration::from_secs(1) + SETTLED_FOR), Some("stopped changing"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
