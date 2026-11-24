use std::collections::HashMap;

use chrono::{DateTime, Utc};
use multitasker_core::{
    Attachment, AttachmentKind, NewAttachment, NewTask, Task, TaskChanges, TaskId, TaskStatus, TaskView, WorkSession, LeftOff,
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

use crate::detection::{self, Snapshot};
use std::sync::Arc;

use multitasker_sync::TokenSource;

use crate::cloud::{self, BuiltInClients, Cloud};
use crate::settings::SyncTarget;
use crate::syncing::{self, FolderSuggestion, SyncStatus};
use crate::tokens::{self, Service};
use crate::{device::Device, settings, wilsoon, windows, AppState};

/// Tells every window the task list changed, so the widget and main window stay in step.
pub const TASKS_CHANGED: &str = "tasks-changed";

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn changed(app: &AppHandle) {
    let _ = app.emit(TASKS_CHANGED, ());
}

#[tauri::command]
pub async fn list_tasks(state: State<'_, AppState>) -> CmdResult<Vec<TaskView>> {
    task_views(&state).await
}

/// Every task with MultiTasker's extras joined in, from the task list itself when it keeps them.
pub async fn task_views(state: &AppState) -> CmdResult<Vec<TaskView>> {
    let provider = state.provider();
    let tasks = provider.list().await.map_err(err)?;
    let key = provider.key();
    let mut handed_in = state.deliverables.counts(key).map_err(err)?;
    let mut worked = state.state.worked(key).await.map_err(err)?;
    let mut left_off = state.state.left_off(key).await.map_err(err)?;
    let mut checklists = state.state.checklists(key).await.map_err(err)?;

    if let Some(store) = provider.extras() {
        let mut extras = store.extras().await.map_err(err)?;
        return Ok(tasks
            .into_iter()
            .map(|task| {
                let e = extras.remove(&task.id).unwrap_or_default();
                // Lists that only keep the computer's name still know it was started here.
                let here = e.started_on_name.as_deref() == Some(state.device.name.as_str());
                let started_on = e.started_on.or_else(|| here.then(|| state.device.id.clone()));
                TaskView {
                    in_progress: e.started_at.is_some(),
                    started_at: e.started_at,
                    in_progress_on: started_on.into_iter().collect(),
                    started_on_name: e.started_on_name,
                    attachments: e.attachments,
                    deliverables: handed_in.remove(&task.id).unwrap_or(0),
                    // Time the list couldn't take, such as before a dashboard kept time, was kept here instead.
                    worked_secs: e.worked_secs + worked.remove(&task.id).unwrap_or(0),
                    left_off: e.left_off.or_else(|| left_off.remove(&task.id)),
                    checklist: e.checklist,
                    task,
                }
            })
            .collect());
    }

    // Earliest start per task across every device: "in progress since" means since anyone picked it up.
    let mut started = HashMap::new();
    let mut devices: HashMap<TaskId, Vec<String>> = HashMap::new();
    for entry in state.state.focus().await.map_err(err)?.into_iter().filter(|e| e.provider == key) {
        devices.entry(entry.task_id.clone()).or_default().push(entry.device);
        let at = started.entry(entry.task_id).or_insert(entry.started_at);
        *at = (*at).min(entry.started_at);
    }

    let mut attachments: HashMap<TaskId, Vec<Attachment>> = HashMap::new();
    for a in state.state.attachments().await.map_err(err)?.into_iter().filter(|a| a.provider == key) {
        attachments.entry(a.task_id.clone()).or_default().push(a);
    }

    Ok(tasks
        .into_iter()
        .map(|task| {
            let started_at = started.get(&task.id).copied();
            let attachments = attachments.remove(&task.id).unwrap_or_default();
            let in_progress_on = devices.remove(&task.id).unwrap_or_default();
            let deliverables = handed_in.remove(&task.id).unwrap_or(0);
            let worked_secs = worked.remove(&task.id).unwrap_or(0);
            let left_off = left_off.remove(&task.id);
            let checklist = checklists.remove(&task.id).unwrap_or_default();
            TaskView {
                task,
                in_progress: started_at.is_some(),
                started_at,
                in_progress_on,
                started_on_name: None,
                attachments,
                deliverables,
                worked_secs,
                left_off,
                checklist,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn create_task(app: AppHandle, state: State<'_, AppState>, task: NewTask) -> CmdResult<Task> {
    let task = state.provider().create(task).await.map_err(err)?;
    changed(&app);
    Ok(task)
}

#[tauri::command]
pub async fn update_task(
    app: AppHandle,
    state: State<'_, AppState>,
    id: TaskId,
    changes: TaskChanges,
) -> CmdResult<Task> {
    let provider = state.provider();
    let task = provider.update(&id, changes).await.map_err(err)?;
    // A finished task is no longer in progress anywhere; a task list that keeps extras clears that itself.
    if task.status == TaskStatus::Done {
        state.projects.stop_with_task(&app, &id);
        end_work(&state, &id, Utc::now()).await?;
        if provider.extras().is_none() {
            state.state.stop(provider.key(), &id).await.map_err(err)?;
        }
    }
    changed(&app);
    Ok(task)
}

#[tauri::command]
pub async fn delete_task(app: AppHandle, state: State<'_, AppState>, id: TaskId) -> CmdResult<()> {
    let provider = state.provider();
    provider.delete(&id).await.map_err(err)?;
    if provider.extras().is_none() {
        state.state.forget(provider.key(), &id).await.map_err(err)?;
    }
    state.deliverables.forget_task(provider.key(), &id, Some(&state.device.id)).map_err(err)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_in_progress(
    app: AppHandle,
    state: State<'_, AppState>,
    id: TaskId,
    in_progress: bool,
) -> CmdResult<()> {
    let provider = state.provider();
    let key = provider.key();
    if in_progress {
        // Starting a finished task reopens it.
        let task = provider.list().await.map_err(err)?.into_iter().find(|t| t.id == id);
        match task {
            None => return Err(format!("task {id} was not found")),
            Some(t) if t.status == TaskStatus::Done => {
                let reopen = TaskChanges { status: Some(TaskStatus::Todo), ..Default::default() };
                provider.update(&id, reopen).await.map_err(err)?;
            }
            Some(_) => {}
        }
        match provider.extras() {
            Some(store) => store.start(&id, &state.device.id, &state.device.name).await.map_err(err)?,
            None => {
                state.state.start(key, &id, &state.device.id).await.map_err(err)?;
            }
        }
        state.state.begin_work(key, &id, Utc::now()).await.map_err(err)?;
        start_autoruns(&app, &state, &id).await;
        changed(&app);
        Ok(())
    } else {
        stop_task(&app, &state, &id, Utc::now()).await
    }
}

/// Pauses a task, counting the work on it up to `ended_at`, such as when the person left rather than now.
pub async fn stop_task(app: &AppHandle, state: &AppState, id: &TaskId, ended_at: DateTime<Utc>) -> CmdResult<()> {
    state.projects.stop_with_task(app, id);
    let provider = state.provider();
    end_work(state, id, ended_at).await?;
    match provider.extras() {
        Some(store) => store.stop(id).await.map_err(err)?,
        None => state.state.stop(provider.key(), id).await.map_err(err)?,
    }
    changed(app);
    Ok(())
}

/// Keeps the stretch of work this computer began on a task, if there was one.
async fn end_work(state: &AppState, id: &TaskId, ended_at: DateTime<Utc>) -> CmdResult<()> {
    let provider = state.provider();
    let key = provider.key();
    let began = state.state.end_work(key, id).await.map_err(err)?;
    // Tasks started before time was kept still count from when this computer started them.
    let began = match began {
        Some(at) => Some(at),
        None if provider.extras().is_none() => state
            .state
            .focus()
            .await
            .map_err(err)?
            .into_iter()
            .find(|f| f.provider == key && &f.task_id == id && f.device == state.device.id)
            .map(|f| f.started_at),
        None => None,
    };
    let Some(started_at) = began.filter(|at| *at < ended_at) else { return Ok(()) };
    // A task list that keeps extras keeps the time too, so it follows the task to other computers.
    let mut in_list = false;
    if let Some(store) = provider.extras() {
        match store.add_worked(id, (ended_at - started_at).num_seconds()).await {
            Ok(()) => in_list = true,
            Err(e) => eprintln!("{} couldn't keep the time on {id}, so it's kept here: {e}", provider.display_name()),
        }
    }
    // Every stretch is kept here as well, for seeing where time went day by day.
    {
        let session = WorkSession {
            id: uuid::Uuid::new_v4().to_string(),
            provider: key.to_owned(),
            task_id: id.clone(),
            device: state.device.id.clone(),
            started_at,
            ended_at,
            in_list,
        };
        state.state.log_work(session).await.map_err(err)?;
    }
    Ok(())
}

/// Attaches what it can; problems with the rest come back as one error listing them.
#[tauri::command]
pub async fn add_attachments(
    app: AppHandle,
    state: State<'_, AppState>,
    id: TaskId,
    items: Vec<NewAttachment>,
) -> CmdResult<Vec<Attachment>> {
    if !state.provider().list().await.map_err(err)?.iter().any(|t| t.id == id) {
        return Err(format!("task {id} was not found"));
    }
    attach(&app, &state, id, items).await
}

/// Adds attachments to a task already known to exist, such as one just created.
pub async fn attach(app: &AppHandle, state: &AppState, id: TaskId, items: Vec<NewAttachment>) -> CmdResult<Vec<Attachment>> {
    let provider = state.provider();
    let key = provider.key();
    let (mut added, mut problems) = (Vec::new(), Vec::new());
    for item in items {
        let attachment = match Attachment::new(key, &id, &state.device.id, item) {
            Ok(a) => a,
            Err(e) => {
                problems.push(e.to_string());
                continue;
            }
        };
        let stored = match provider.extras() {
            Some(store) => store.add_attachment(attachment).await,
            None => state.state.add_attachment(attachment).await,
        };
        match stored {
            Ok(a) => added.push(a),
            Err(e) => problems.push(e.to_string()),
        }
    }

    if !added.is_empty() {
        changed(app);
    }
    if problems.is_empty() {
        Ok(added)
    } else {
        Err(problems.join(" "))
    }
}

#[tauri::command]
pub async fn remove_attachment(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let provider = state.provider();
    match provider.extras() {
        Some(store) => store.remove_attachment(&id).await.map_err(err)?,
        None => state.state.remove_attachment(&id).await.map_err(err)?,
    }
    changed(&app);
    Ok(())
}

/// An attachment on any task in the current list, wherever its extras are kept.
async fn find_attachment(state: &AppState, id: &str) -> CmdResult<Attachment> {
    task_views(state)
        .await?
        .into_iter()
        .flat_map(|t| t.attachments)
        .find(|a| a.id == id)
        .ok_or_else(|| "That attachment no longer exists.".to_owned())
}

/// Checks a file attachment is still where it was, explaining why not if it isn't.
fn existing_path(state: &AppState, a: &Attachment) -> CmdResult<std::path::PathBuf> {
    let path = std::path::PathBuf::from(&a.target);
    if path.exists() {
        return Ok(path);
    }
    Err(match &a.device {
        Some(device) if device != &state.device.id => format!("'{}' was attached on another computer.", a.name),
        _ => format!("'{}' has been moved or deleted ({}).", a.name, a.target),
    })
}

/// Notes where work on a task stopped; blank clears the note.
#[tauri::command]
pub async fn set_left_off(app: AppHandle, state: State<'_, AppState>, task_id: TaskId, text: String) -> CmdResult<()> {
    let text = text.trim();
    let note = (!text.is_empty()).then(|| LeftOff {
        text: text.chars().take(500).collect(),
        at: Utc::now(),
        device: state.device.id.clone(),
    });
    let provider = state.provider();
    let kept_by_list = match provider.extras() {
        Some(store) => match store.set_left_off(&task_id, note.clone()).await {
            Ok(()) => true,
            Err(e) => {
                eprintln!("{} couldn't keep the note on {task_id}, so it's kept here: {e}", provider.display_name());
                false
            }
        },
        None => false,
    };
    // The local copy goes once the list has it, so it never shadows a newer one written elsewhere.
    state.state.set_left_off(provider.key(), &task_id, if kept_by_list { None } else { note }).await.map_err(err)?;
    changed(&app);
    Ok(())
}

/// What opening a task's things managed, for the UI to report.
#[derive(serde::Serialize)]
pub struct Reopened {
    pub opened: usize,
    /// Things that couldn't be opened here, with why.
    pub skipped: Vec<String>,
}

/// Adds a step to a task's checklist, in the task list when it keeps them.
#[tauri::command]
pub async fn add_check(app: AppHandle, state: State<'_, AppState>, task_id: TaskId, text: String) -> CmdResult<()> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A step needs some words.".into());
    }
    let text: String = text.chars().take(300).collect();
    let provider = state.provider();
    match provider.extras() {
        Some(store) => {
            store.add_check(&task_id, &text).await.map_err(err)?;
        }
        None => {
            let item = multitasker_core::CheckItem { id: uuid::Uuid::new_v4().to_string(), text, done: false, added_at: Utc::now() };
            state.state.add_check(provider.key(), &task_id, item).await.map_err(err)?;
        }
    }
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_check(app: AppHandle, state: State<'_, AppState>, task_id: TaskId, id: String, done: bool) -> CmdResult<()> {
    match state.provider().extras() {
        Some(store) => store.set_check(&task_id, &id, done).await.map_err(err)?,
        None => state.state.set_check(&id, done).await.map_err(err)?,
    }
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn remove_check(app: AppHandle, state: State<'_, AppState>, task_id: TaskId, id: String) -> CmdResult<()> {
    match state.provider().extras() {
        Some(store) => store.remove_check(&task_id, &id).await.map_err(err)?,
        None => state.state.remove_check(&id).await.map_err(err)?,
    }
    changed(&app);
    Ok(())
}

/// What starting a task here can open: files and folders found on this computer with a program for them, and pages.
#[tauri::command]
pub async fn task_openable(state: State<'_, AppState>, task_id: TaskId) -> CmdResult<crate::here::Openable> {
    let task = task_views(&state).await?.into_iter().find(|t| t.task.id == task_id);
    let attachments = task.map(|t| t.attachments).unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || crate::here::openable(&attachments).0).await.map_err(err)
}

/// Opens a task's pages, and its files and folders that are on this computer, to pick it up again.
#[tauri::command]
pub async fn open_task_things(app: AppHandle, state: State<'_, AppState>, task_id: TaskId) -> CmdResult<Reopened> {
    let task = task_views(&state)
        .await?
        .into_iter()
        .find(|t| t.task.id == task_id)
        .ok_or_else(|| format!("task {task_id} was not found"))?;
    let (found, paths) = crate::here::openable(&task.attachments);
    let mut report = Reopened { opened: 0, skipped: found.unavailable };
    // With the extension connected, the browser switches to pages already open and opens the rest beside them.
    let urls: Vec<String> = task.attachments.iter().filter(|a| a.kind == AttachmentKind::Link).map(|a| a.target.clone()).collect();
    if !urls.is_empty() && crate::bridge::open_tabs(&urls) {
        report.opened += urls.len();
    } else {
        for url in &urls {
            match app.opener().open_url(url, None::<&str>) {
                Ok(()) => report.opened += 1,
                Err(e) => report.skipped.push(e.to_string()),
            }
        }
    }
    for path in paths {
        match app.opener().open_path(path.to_string_lossy(), None::<&str>) {
            Ok(()) => report.opened += 1,
            Err(e) => report.skipped.push(e.to_string()),
        }
    }
    Ok(report)
}

/// Opens an attachment in its default app; done here so the webview can only open what was attached.
#[tauri::command]
pub async fn open_attachment(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let a = find_attachment(&state, &id).await?;
    match a.kind {
        AttachmentKind::Link => app.opener().open_url(&a.target, None::<&str>).map_err(err),
        _ => app.opener().open_path(existing_path(&state, &a)?.to_string_lossy(), None::<&str>).map_err(err),
    }
}

/// Shows a file attachment selected in its folder.
#[tauri::command]
pub async fn reveal_attachment(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let a = find_attachment(&state, &id).await?;
    if a.kind == AttachmentKind::Link {
        return Err("Links have no folder to show.".into());
    }
    app.opener().reveal_item_in_dir(existing_path(&state, &a)?).map_err(err)
}

fn work_record(provider: &str, f: &multitasker_core::HandedInFile) -> multitasker_deliverables::Record {
    multitasker_deliverables::Record {
        id: f.id.clone(),
        provider: provider.to_owned(),
        task_id: f.task_id.clone(),
        name: f.name.clone(),
        sha256: f.sha256.clone(),
        size: f.size,
        source: String::new(),
        device: f.device.clone(),
        added_at: f.added_at,
    }
}

fn work_file(r: &multitasker_deliverables::Record) -> multitasker_core::HandedInFile {
    multitasker_core::HandedInFile {
        id: r.id.clone(),
        task_id: r.task_id.clone(),
        name: r.name.clone(),
        sha256: r.sha256.clone(),
        size: r.size,
        device: r.device.clone(),
        added_at: r.added_at,
    }
}

/// The work handed in for a task, each with its revisions; read fresh from the task list when it keeps it.
#[tauri::command]
pub async fn list_deliverables(
    state: State<'_, AppState>,
    task_id: TaskId,
) -> CmdResult<Vec<multitasker_deliverables::Deliverable>> {
    let provider = state.provider();
    let key = provider.key().to_owned();
    if let Some(work) = provider.work() {
        // Offline, what's known here is still shown.
        if let Ok(files) = work.handed_in(&task_id).await {
            let records: Vec<_> = files.iter().map(|f| work_record(&key, f)).collect();
            state.deliverables.replace_task(&key, &task_id, &records).map_err(err)?;
        }
    }
    state.deliverables.list(&key, &task_id).map_err(err)
}

/// Keeps copies of files as the task's deliverables; a file handed in again becomes a new revision.
#[tauri::command]
pub async fn hand_in(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: TaskId,
    paths: Vec<String>,
) -> CmdResult<Vec<multitasker_deliverables::HandedIn>> {
    let provider = state.provider();
    let key = provider.key().to_owned();
    if !provider.list().await.map_err(err)?.iter().any(|t| t.id == task_id) {
        return Err("That task no longer exists.".into());
    }
    let store = state.deliverables.clone();
    let mut handed_in = Vec::new();
    let mut problems = Vec::new();
    for path in paths {
        // Copying big files takes a moment, so it happens off the UI's thread.
        let kept = {
            let (store, path) = (store.clone(), path.trim().to_owned());
            tauri::async_runtime::spawn_blocking(move || store.keep(std::path::Path::new(&path))).await.map_err(err)?
        };
        let kept = match kept {
            Ok(k) => k,
            Err(e) => {
                problems.push(e.to_string());
                continue;
            }
        };
        if let Some(same) = store.unchanged(&key, &task_id, &kept).map_err(err)? {
            handed_in.push(same);
            continue;
        }
        let mut record = kept.record(uuid::Uuid::new_v4().to_string(), &key, &task_id, Some(&state.device.id));
        // A task list that keeps handed-in work gets the file itself, and its id names the revision.
        if let Some(work) = provider.work() {
            let bytes = match store.read_copy(&kept.sha256) {
                Ok(b) => b,
                Err(e) => {
                    problems.push(e.to_string());
                    continue;
                }
            };
            match work.hand_in(work_file(&record), bytes, &state.device.name).await {
                Ok(stored) => record.id = stored.id,
                Err(e) => {
                    problems.push(format!("{}: {e}", kept.name));
                    continue;
                }
            }
        }
        match store.add_record(&record) {
            Ok(h) => handed_in.push(h),
            Err(e) => problems.push(e.to_string()),
        }
    }
    if !handed_in.is_empty() {
        // Hand-ins aren't task changes, so sync is woken for them directly.
        state.syncer.wake_soon();
        changed(&app);
    }
    if problems.is_empty() {
        Ok(handed_in)
    } else {
        Err(problems.join(" "))
    }
}

/// Makes sure a revision's copy is here, fetching it from the task list if that's where it is.
async fn ensure_copy(state: &AppState, revision_id: &str) -> CmdResult<()> {
    let record = state.deliverables.record(revision_id).map_err(err)?;
    if state.deliverables.has(&record.sha256) {
        return Ok(());
    }
    let provider = state.provider();
    let Some(work) = provider.work().filter(|_| record.provider == provider.key()) else { return Ok(()) };
    let bytes = work.download(&work_file(&record)).await.map_err(err)?;
    if !state.deliverables.receive_copy(&record.sha256, &bytes).map_err(err)? {
        return Err("The downloaded file doesn't match what was handed in.".into());
    }
    Ok(())
}

/// Opens a read-only copy of a revision, so the kept one can't be changed by accident.
#[tauri::command]
pub async fn open_deliverable(app: AppHandle, state: State<'_, AppState>, revision_id: String) -> CmdResult<()> {
    ensure_copy(&state, &revision_id).await?;
    let path = state.deliverables.open_copy(&revision_id).map_err(err)?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(err)
}

/// Saves a copy of a revision where the person chose.
#[tauri::command]
pub async fn export_deliverable(state: State<'_, AppState>, revision_id: String, dest: String) -> CmdResult<()> {
    ensure_copy(&state, &revision_id).await?;
    let store = state.deliverables.clone();
    tauri::async_runtime::spawn_blocking(move || store.export(&revision_id, std::path::Path::new(&dest)))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn remove_deliverable(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let provider = state.provider();
    if let Some(work) = provider.work() {
        for revision in state.deliverables.revision_ids(&id).map_err(err)? {
            work.remove_handed_in(&revision).await.map_err(err)?;
        }
    }
    state.deliverables.remove(&id, Some(&state.device.id)).map_err(err)?;
    state.syncer.wake_soon();
    changed(&app);
    Ok(())
}

/// Brings in work handed in on other computers to a task list that keeps it, a few copies at a time.
pub async fn catch_up_work(state: &AppState) -> CmdResult<bool> {
    let provider = state.provider();
    let Some(work) = provider.work() else { return Ok(false) };
    let key = provider.key().to_owned();
    let records: Vec<_> = work.all_handed_in().await.map_err(err)?.iter().map(|f| work_record(&key, f)).collect();
    let mut changed = state.deliverables.merge(&records, &[]).map_err(err)? > 0;
    for record in state.deliverables.missing(&key).map_err(err)?.into_iter().take(5) {
        if let Ok(bytes) = work.download(&work_file(&record)).await {
            changed |= state.deliverables.receive_copy(&record.sha256, &bytes).map_err(err)?;
        }
    }
    Ok(changed)
}

#[tauri::command]
pub fn this_device(state: State<'_, AppState>) -> Device {
    state.device.clone()
}

#[tauri::command]
pub fn open_main(app: AppHandle) -> CmdResult<()> {
    windows::show_main(&app).map_err(err)
}

#[tauri::command]
pub fn widget_visible(app: AppHandle) -> CmdResult<bool> {
    windows::widget_visible(&app).map_err(err)
}

#[tauri::command]
pub fn set_widget_visible(app: AppHandle, visible: bool) -> CmdResult<bool> {
    windows::set_widget_visible(&app, visible).map_err(err)
}

/// A program with a window open, offered when linking an app to a task.
#[derive(serde::Serialize)]
pub struct RunningApp {
    exe_path: String,
    exe: String,
    name: String,
    title: String,
}

#[tauri::command]
pub async fn running_apps() -> Vec<RunningApp> {
    // Reading every window's program details takes a moment; keep it off the main thread.
    tauri::async_runtime::spawn_blocking(|| {
        multitasker_activity::app_windows()
            .into_iter()
            .map(|a| RunningApp { exe_path: a.exe_path, exe: a.exe, name: a.name, title: a.title })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub fn activity(state: State<'_, AppState>) -> Snapshot {
    state.detector.snapshot()
}

#[tauri::command]
pub fn dismiss_suggestion(app: AppHandle, state: State<'_, AppState>, task_id: TaskId) {
    state.detector.dismiss(&app, &task_id);
}

#[tauri::command]
pub fn set_detection(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    settings::update(&state.data_dir, |s| s.detection = enabled).map_err(err)?;
    state.detector.set_enabled(&app, enabled);
    detection::sync_tray(&app, enabled);
    Ok(())
}

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    state.syncer.status()
}

/// Chooses where to sync (or turns it off) and starts a round right away. A new S3 secret goes to the credential store.
#[tauri::command]
pub async fn set_sync_target(
    app: AppHandle,
    state: State<'_, AppState>,
    target: SyncTarget,
    secret: Option<String>,
) -> CmdResult<()> {
    match &target {
        SyncTarget::Off => state.db.disable_sync().map_err(err)?,
        SyncTarget::Folder { path } => syncing::check_folder(path)?,
        SyncTarget::S3(config) => {
            if let Some(secret) = secret.as_deref().filter(|s| !s.is_empty()) {
                syncing::save_s3_secret(config, secret)?;
            }
            let config = config.clone();
            // A network round trip; keep it off the main thread.
            tauri::async_runtime::spawn_blocking(move || syncing::s3_remote(&config).and_then(|r| syncing::check_s3(&r)))
                .await
                .map_err(err)??;
        }
        SyncTarget::GoogleDrive { .. } | SyncTarget::OneDrive { .. } => {
            return Err("Sign in from Settings to sync with Google Drive or OneDrive.".into());
        }
    }

    let previous = settings::load(&state.data_dir).sync;
    settings::update(&state.data_dir, |s| s.sync = target.clone()).map_err(err)?;
    moved(&state, &previous, &target)?;
    forget_replaced(&previous, &target);
    state.syncer.retarget(&app, target);
    state.syncer.wake();
    Ok(())
}

/// A new place to sync (another service, account, folder or bucket) starts empty, so everything is uploaded again.
fn moved(state: &AppState, previous: &SyncTarget, next: &SyncTarget) -> CmdResult<()> {
    if previous != next {
        state.db.reset_sync().map_err(err)?;
    }
    Ok(())
}

/// Removes saved secrets the new sync target no longer needs: an old S3 key, or a sign-in to another service.
fn forget_replaced(previous: &SyncTarget, next: &SyncTarget) {
    if let SyncTarget::S3(old) = previous {
        if !matches!(next, SyncTarget::S3(new) if new.access_key_id == old.access_key_id && new.endpoint == old.endpoint) {
            syncing::forget_s3_secret(old);
        }
    }
    if let Some((old, _)) = Cloud::of(previous) {
        if Cloud::of(next).map(|(c, _)| c) != Some(old) {
            old.sign_out();
        }
    }
}

/// Signs in to Google or Microsoft in the browser, checks the storage answers, and syncs there from now on.
#[tauri::command]
pub async fn sign_in_cloud(
    app: AppHandle,
    state: State<'_, AppState>,
    service: String,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> CmdResult<String> {
    let cloud = Cloud::parse(&service)?;
    let client_id = client_id.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
    if let Some(secret) = client_secret.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cloud.save_client_secret(secret)?;
    }
    let oauth = cloud.app(client_id.as_deref())?;
    let cancel = state.sign_in_cancel.clone();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let opener = app.clone();
    let (signed_in, tokens) = tauri::async_runtime::spawn_blocking(move || {
        let open = |url: &str| opener.opener().open_url(url, None::<&str>).map_err(|e| e.to_string());
        let signed_in = multitasker_sync::sign_in(&oauth, open, &cancel).map_err(err)?;
        cloud.save_refresh_token(&signed_in.refresh_token)?;
        let tokens = Arc::new(
            TokenSource::new(oauth, signed_in.refresh_token.clone(), move |rotated| {
                let _ = cloud.save_refresh_token(rotated);
            })
            .with_access_token(signed_in.access_token.clone(), signed_in.expires_in),
        );
        // One request now, so a storage problem shows up here rather than on the next sync.
        syncing::cloud_remote(cloud, tokens.clone()).list("multitasker/v1/devices/").map_err(err)?;
        Ok::<_, String>((signed_in, tokens))
    })
    .await
    .map_err(err)??;

    let account = signed_in.account.unwrap_or_else(|| format!("your {} account", cloud.label()));
    let target = match cloud {
        Cloud::Google => SyncTarget::GoogleDrive { account: account.clone(), client_id },
        Cloud::Microsoft => SyncTarget::OneDrive { account: account.clone(), client_id },
    };
    let previous = settings::load(&state.data_dir).sync;
    settings::update(&state.data_dir, |s| s.sync = target.clone()).map_err(err)?;
    moved(&state, &previous, &target)?;
    if Cloud::of(&previous).map(|(c, _)| c) != Some(cloud) {
        forget_replaced(&previous, &target);
    }
    state.syncer.use_tokens(target.clone(), tokens);
    state.syncer.retarget(&app, target);
    state.syncer.wake();
    Ok(account)
}

#[tauri::command]
pub fn cancel_sign_in(state: State<'_, AppState>) {
    state.sign_in_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
pub fn built_in_clients() -> BuiltInClients {
    cloud::built_in_clients()
}

#[tauri::command]
pub fn sync_now(state: State<'_, AppState>) {
    state.syncer.wake();
}

#[tauri::command]
pub fn sync_folder_suggestions() -> Vec<FolderSuggestion> {
    syncing::folder_suggestions()
}

#[tauri::command]
pub fn detection_enabled(state: State<'_, AppState>) -> bool {
    state.detector.enabled()
}

/// Starts a task and pauses what this computer was working on before.
#[tauri::command]
pub async fn switch_task(app: AppHandle, state: State<'_, AppState>, task_id: TaskId) -> CmdResult<()> {
    detection::switch_to(&app, &state, &task_id).await?;
    Ok(())
}

/// Goes back to what was in progress before detection switched on its own, and stays off that task for a while.
#[tauri::command]
pub async fn undo_switch(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let Some(switched) = state.detector.take_switch(&app) else { return Ok(()) };
    for t in &switched.from {
        set_in_progress(app.clone(), state.clone(), t.task_id.clone(), true).await?;
    }
    set_in_progress(app.clone(), state.clone(), switched.to.task_id.clone(), false).await?;
    state.detector.dismiss(&app, &switched.to.task_id);
    Ok(())
}

#[tauri::command]
pub fn dismiss_switch(app: AppHandle, state: State<'_, AppState>) {
    state.detector.take_switch(&app);
}

/// Seconds a task's related file or program must stay in front before it starts on its own.
#[tauri::command]
pub fn auto_start_after(state: State<'_, AppState>) -> u64 {
    state.detector.start_after().as_secs()
}

#[tauri::command]
pub fn set_auto_start_after(state: State<'_, AppState>, secs: u64) -> CmdResult<()> {
    let secs = secs.clamp(1, 3600);
    settings::update(&state.data_dir, |s| s.auto_start_after_secs = secs).map_err(err)?;
    state.detector.set_start_after(secs);
    Ok(())
}

#[tauri::command]
pub fn auto_switch(state: State<'_, AppState>) -> bool {
    state.detector.auto_switch()
}

#[tauri::command]
pub fn set_auto_switch(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    settings::update(&state.data_dir, |s| s.auto_switch = enabled).map_err(err)?;
    state.detector.set_auto_switch(enabled);
    Ok(())
}

/// Links the page detection offered to the task in progress.
#[tauri::command]
pub async fn accept_link_offer(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let Some(offer) = state.detector.take_link_offer(&app) else { return Ok(()) };
    let item = NewAttachment::Link { url: offer.url, name: offer.title.filter(|t| !t.trim().is_empty()) };
    attach(&app, &state, offer.task_id, vec![item]).await?;
    Ok(())
}

#[tauri::command]
pub fn refuse_link_offer(app: AppHandle, state: State<'_, AppState>) {
    state.detector.refuse_link_offer(&app);
}

#[tauri::command]
pub fn widget_on_top(state: State<'_, AppState>) -> settings::OnTop {
    settings::load(&state.data_dir).widget_on_top
}

#[tauri::command]
pub fn set_widget_on_top(state: State<'_, AppState>, on_top: settings::OnTop) -> CmdResult<()> {
    settings::update(&state.data_dir, |s| s.widget_on_top = on_top.clone()).map_err(err)?;
    crate::on_top::set(on_top);
    Ok(())
}

/// Looks files over before they're handed in: open in another program, empty, misnamed or unchanged since last time.
#[tauri::command]
pub async fn check_hand_in(
    state: State<'_, AppState>,
    task_id: TaskId,
    paths: Vec<String>,
) -> CmdResult<Vec<crate::checks::FileCheck>> {
    let key = state.provider().key().to_owned();
    let store = state.deliverables.clone();
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .iter()
            .map(|p| {
                let path = std::path::Path::new(p.trim());
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let last = store.latest_sha(&key, &task_id, &name).ok().flatten();
                crate::checks::check_file(path, last.as_deref())
            })
            .collect()
    })
    .await
    .map_err(err)
}

/// Opens a link the way someone without your sign-in would, to tell whether others can see it.
#[tauri::command]
pub async fn check_link(url: String) -> CmdResult<crate::checks::LinkCheck> {
    tauri::async_runtime::spawn_blocking(move || crate::checks::check_link(&url)).await.map_err(err)
}

/// Hands in a link, kept as an internet shortcut so it copies, syncs and opens like any handed-in file.
#[tauri::command]
pub async fn hand_in_link(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: TaskId,
    url: String,
    title: Option<String>,
) -> CmdResult<Vec<multitasker_deliverables::HandedIn>> {
    let url = url.trim();
    let url = if url.contains("://") { url.to_owned() } else { format!("https://{url}") };
    let dir = std::env::temp_dir().join(format!("multitasker-link-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(err)?;
    let path = dir.join(crate::checks::shortcut_name(&url, title.as_deref()));
    std::fs::write(&path, crate::checks::shortcut(&url)).map_err(err)?;
    let handed = hand_in(app, state, task_id, vec![path.display().to_string()]).await;
    let _ = std::fs::remove_dir_all(&dir);
    handed
}

#[tauri::command]
pub fn update_info(app: AppHandle) -> crate::updates::UpdateInfo {
    crate::updates::info(&app)
}

/// Looks for a newer version now; returns whether one is downloaded and ready.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> CmdResult<bool> {
    if crate::updates::private_build() {
        return Err("This private build doesn't update itself; install new versions by hand.".into());
    }
    if !crate::updates::enabled() {
        return Err("Development builds don't update themselves.".into());
    }
    crate::updates::check(&app).await
}

#[tauri::command]
pub fn install_update(app: AppHandle) -> CmdResult<()> {
    crate::updates::install(&app)
}

#[tauri::command]
pub fn start_with_windows(state: State<'_, AppState>) -> bool {
    settings::load(&state.data_dir).start_with_windows
}

#[tauri::command]
pub fn set_start_with_windows(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    crate::updates::apply_autostart(&app, enabled)?;
    settings::update(&state.data_dir, |s| s.start_with_windows = enabled).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn hide_switcher(app: AppHandle) {
    crate::switcher::hide(&app);
}

/// Why the switcher's hotkey isn't working, if it isn't.
#[tauri::command]
pub fn switcher_problem() -> Option<String> {
    crate::switcher::problem()
}

#[tauri::command]
pub fn switcher_shortcut(state: State<'_, AppState>) -> Option<String> {
    settings::load(&state.data_dir).switcher_shortcut
}

/// Changes the switcher's hotkey, or turns it off with `None`; a hotkey another program has is refused.
#[tauri::command]
pub fn set_switcher_shortcut(app: AppHandle, state: State<'_, AppState>, shortcut: Option<String>) -> CmdResult<()> {
    let shortcut = shortcut.filter(|s| !s.trim().is_empty());
    if let Err(e) = crate::switcher::set_shortcut(&app, shortcut.as_deref()) {
        let old = settings::load(&state.data_dir).switcher_shortcut;
        let _ = crate::switcher::set_shortcut(&app, old.as_deref());
        return Err(e);
    }
    settings::update(&state.data_dir, |s| s.switcher_shortcut = shortcut).map_err(err)?;
    Ok(())
}

/// Finished stretches of work between two moments, for the Time view.
#[tauri::command]
pub async fn work_sessions(
    state: State<'_, AppState>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> CmdResult<Vec<WorkSession>> {
    state.state.sessions(state.provider().key(), from, to).await.map_err(err)
}

/// What the task's linked folders on this computer can run, from their package.json, Cargo.toml and the like.
#[tauri::command]
pub async fn project_commands(state: State<'_, AppState>, task_id: TaskId) -> CmdResult<Vec<crate::projects::ProjectCommand>> {
    let task = task_views(&state).await?.into_iter().find(|t| t.task.id == task_id);
    let Some(task) = task else { return Ok(Vec::new()) };
    let folders = crate::projects::task_folders(&task.attachments, &state.device.id);
    Ok(folders.iter().flat_map(|f| crate::projects::detect(f)).collect())
}

#[tauri::command]
pub fn project_runs(state: State<'_, AppState>) -> Vec<crate::projects::RunView> {
    state.projects.views()
}

/// Runs one of a task's project commands, found again here so the window can only run what a folder offers.
#[tauri::command]
pub async fn run_project_command(app: AppHandle, state: State<'_, AppState>, task_id: TaskId, command_id: String) -> CmdResult<()> {
    let found = project_commands(state.clone(), task_id.clone()).await?.into_iter().find(|c| c.id == command_id);
    let command = found.ok_or("That command isn't in the task's folders any more.")?;
    state.projects.start(&app, &task_id, command, false)
}

#[tauri::command]
pub fn stop_project_command(app: AppHandle, state: State<'_, AppState>, command_id: String) {
    state.projects.stop(&app, &command_id);
}

/// Opens the local address a running command printed.
#[tauri::command]
pub fn open_project_url(app: AppHandle, state: State<'_, AppState>, command_id: String) -> CmdResult<()> {
    let url = state.projects.url(&command_id).ok_or("It hasn't said where it's running yet.")?;
    app.opener().open_url(url, None::<&str>).map_err(err)
}

/// Marks a project command to start with its task and stop when it pauses, or not.
#[tauri::command]
pub fn set_project_autorun(state: State<'_, AppState>, task_id: TaskId, command_id: String, on: bool) -> CmdResult<()> {
    settings::update(&state.data_dir, |s| {
        let list = s.project_autoruns.entry(task_id.clone()).or_default();
        list.retain(|c| c != &command_id);
        if on {
            list.push(command_id.clone());
        }
        if list.is_empty() {
            s.project_autoruns.remove(&task_id);
        }
    })
    .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn project_autoruns(state: State<'_, AppState>, task_id: TaskId) -> Vec<String> {
    settings::load(&state.data_dir).project_autoruns.remove(&task_id).unwrap_or_default()
}

/// Starts the project commands marked to run along with the task.
async fn start_autoruns(app: &AppHandle, state: &AppState, task_id: &TaskId) {
    let wanted = settings::load(&state.data_dir).project_autoruns.remove(task_id).unwrap_or_default();
    if wanted.is_empty() {
        return;
    }
    let Ok(tasks) = task_views(state).await else { return };
    let Some(task) = tasks.into_iter().find(|t| &t.task.id == task_id) else { return };
    for folder in crate::projects::task_folders(&task.attachments, &state.device.id) {
        for command in crate::projects::detect(&folder).into_iter().filter(|c| wanted.contains(&c.id)) {
            if let Err(e) = state.projects.start(app, task_id, command, true) {
                eprintln!("{e}");
            }
        }
    }
}

#[tauri::command]
pub fn dismiss_paused(app: AppHandle, state: State<'_, AppState>) {
    state.detector.dismiss_paused(&app);
}

#[tauri::command]
pub fn pause_after_away(state: State<'_, AppState>) -> Option<u32> {
    settings::load(&state.data_dir).pause_after_away
}

#[tauri::command]
pub fn set_pause_after_away(state: State<'_, AppState>, minutes: Option<u32>) -> CmdResult<()> {
    settings::update(&state.data_dir, |s| s.pause_after_away = minutes.filter(|m| *m > 0)).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn reminder_settings(state: State<'_, AppState>) -> settings::Reminders {
    settings::load(&state.data_dir).reminders
}

#[tauri::command]
pub fn set_reminder_settings(state: State<'_, AppState>, reminders: settings::Reminders) -> CmdResult<()> {
    settings::update(&state.data_dir, |s| s.reminders = reminders).map_err(err)?;
    Ok(())
}

/// Where tasks come from, for Settings.
#[derive(serde::Serialize)]
pub struct TaskSourceView {
    #[serde(flatten)]
    source: settings::TaskSource,
    /// The provider in use (it can fall back to local tasks if a sign-in is gone).
    active: String,
    supports_due_time: bool,
    /// Whether this build can sign in to WilsoonID.
    wilsoon_available: bool,
}

#[tauri::command]
pub fn task_source(state: State<'_, AppState>) -> TaskSourceView {
    let provider = state.provider();
    let mut source = settings::load(&state.data_dir).tasks;
    // A build without a WilsoonID registration (the public one) never offers JavaDashboard. A choice of it made in a
    // private build sharing these settings shows as tasks on this computer, which is what's in use, and is left as it
    // is for the private build.
    if matches!(source, settings::TaskSource::JavaDashboard { .. }) && !wilsoon::is_available() {
        source = settings::TaskSource::Local;
    }
    TaskSourceView {
        source,
        active: provider.key().to_owned(),
        supports_due_time: provider.supports_due_time(),
        wilsoon_available: wilsoon::is_available(),
    }
}

/// Signs in to WilsoonID in the browser and takes tasks from JavaDashboard from now on.
#[tauri::command]
pub async fn sign_in_wilsoon(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let oauth = wilsoon::app()?;
    let pending = multitasker_sync::begin(&oauth, wilsoon::REDIRECT_URI);
    let receiver = state.wilsoon_sign_in.wait_for(&pending);
    let cancel = state.sign_in_cancel.clone();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    app.opener().open_url(&pending.url, None::<&str>).map_err(err)?;

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let url = wilsoon::receive(&receiver, || cancel.load(std::sync::atomic::Ordering::Relaxed))?;
        let code = multitasker_sync::code_from_redirect(&url, pending.state()).map_err(err)?;
        let signed_in = multitasker_sync::finish(&oauth, &pending, &code).map_err(err)?;
        wilsoon::save_refresh_token(&signed_in.refresh_token)?;
        let tokens = wilsoon::tokens()?;
        let provider = wilsoon::provider(tokens);
        // One request now, so a missing delegation shows up here rather than as an empty task list.
        tauri::async_runtime::block_on(multitasker_core::TaskProvider::list(&provider)).map_err(err)?;
        Ok::<_, String>((signed_in.account, provider))
    })
    .await
    .map_err(err)?;
    state.wilsoon_sign_in.clear();
    let (account, provider) = outcome?;

    let account = account.unwrap_or_else(|| "your WilsoonID account".into());
    switch_source(&app, &state, settings::TaskSource::JavaDashboard { account: account.clone() }, Arc::new(provider))?;
    let _ = windows::show_main(&app);
    Ok(account)
}

/// Checks a personal Todoist token and takes tasks from Todoist from now on.
#[tauri::command]
pub async fn connect_todoist(app: AppHandle, state: State<'_, AppState>, token: String) -> CmdResult<String> {
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err("Paste your Todoist API token, from Todoist's Settings → Integrations → Developer.".into());
    }
    let provider = multitasker_provider_todoist::Todoist::new(token.clone());
    let account = provider.account().await.map_err(plain_err)?;
    tokens::save(Service::Todoist, &token)?;
    switch_source(&app, &state, settings::TaskSource::Todoist { account: account.clone() }, Arc::new(provider))?;
    Ok(account)
}

/// Provider errors without their category prefix, for messages shown as they are.
fn plain_err(e: multitasker_core::Error) -> String {
    let text = e.to_string();
    text.split_once(": ").filter(|(head, _)| head.ends_with("error")).map(|(_, rest)| rest.to_owned()).unwrap_or(text)
}

#[derive(serde::Serialize)]
pub struct NotionDatabases {
    workspace: String,
    databases: Vec<NotionDatabase>,
}

#[derive(serde::Serialize)]
pub struct NotionDatabase {
    id: String,
    name: String,
}

/// Checks a Notion integration token and lists the databases shared with it.
#[tauri::command]
pub async fn notion_databases(token: String) -> CmdResult<NotionDatabases> {
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err("Paste your Notion integration's token.".into());
    }
    let (workspace, found) = multitasker_provider_notion::Notion::databases(&token).await.map_err(plain_err)?;
    Ok(NotionDatabases {
        workspace,
        databases: found.into_iter().map(|d| NotionDatabase { id: d.id, name: d.name }).collect(),
    })
}

/// Takes tasks from a Notion database from now on, adding the columns MultiTasker needs to it.
#[tauri::command]
pub async fn connect_notion(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    database: String,
) -> CmdResult<String> {
    let token = token.trim().to_owned();
    let (workspace, found) = multitasker_provider_notion::Notion::databases(&token).await.map_err(plain_err)?;
    let chosen = found
        .into_iter()
        .find(|d| d.id == database)
        .ok_or("MultiTasker can't see that database; share it with your integration in Notion.")?;
    let provider = multitasker_provider_notion::Notion::new(token.clone(), chosen.id.clone());
    provider.prepare().await.map_err(plain_err)?;
    tokens::save(Service::Notion, &token)?;
    let source = settings::TaskSource::Notion { account: workspace, database: chosen.id, name: chosen.name.clone() };
    switch_source(&app, &state, source, Arc::new(provider))?;
    Ok(chosen.name)
}

/// Goes back to the tasks kept on this computer and forgets the online task list's sign-in.
#[tauri::command]
pub fn use_local_tasks(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    switch_source(&app, &state, settings::TaskSource::Local, state.db.clone())
}

/// Moves to another task list, forgetting the sign-in for the one being left.
fn switch_source(
    app: &AppHandle,
    state: &AppState,
    source: settings::TaskSource,
    provider: Arc<dyn multitasker_core::TaskProvider>,
) -> CmdResult<()> {
    let previous = settings::load(&state.data_dir).tasks;
    settings::update(&state.data_dir, |s| s.tasks = source.clone()).map_err(err)?;
    if std::mem::discriminant(&previous) != std::mem::discriminant(&source) {
        match previous {
            settings::TaskSource::JavaDashboard { .. } => wilsoon::sign_out(),
            settings::TaskSource::Todoist { .. } => tokens::forget(Service::Todoist),
            settings::TaskSource::Notion { .. } => tokens::forget(Service::Notion),
            settings::TaskSource::Local => {}
        }
    }
    state.set_provider(provider);
    state.syncer.wake();
    // A list signed in to on a new computer brings the settings chosen on the others.
    crate::shared::wake();
    changed(app);
    Ok(())
}

/// Tells the widget its background opacity changed.
pub const WIDGET_APPEARANCE: &str = "widget-appearance";

#[tauri::command]
pub fn widget_opacity(state: State<'_, AppState>) -> u8 {
    settings::load(&state.data_dir).widget_opacity
}

/// Shows a new opacity on the widget straight away; `save` keeps it (sent when the slider is let go).
#[tauri::command]
pub fn set_widget_opacity(app: AppHandle, state: State<'_, AppState>, opacity: u8, save: bool) -> CmdResult<u8> {
    let opacity = opacity.clamp(settings::MIN_WIDGET_OPACITY, 100);
    if save {
        settings::update(&state.data_dir, |s| s.widget_opacity = opacity).map_err(err)?;
    }
    let _ = app.emit(WIDGET_APPEARANCE, opacity);
    Ok(opacity)
}

/// How sharing settings between computers is going, if they're shared at all.
#[tauri::command]
pub fn settings_sync() -> Option<crate::shared::SettingsSync> {
    crate::shared::status()
}

/// The day's plan, shared between computers.
#[tauri::command]
pub fn day_plan(state: State<'_, AppState>) -> Option<crate::shared::Plan> {
    crate::shared::plan(&state.data_dir)
}

#[tauri::command]
pub fn set_day_plan(app: AppHandle, plan: crate::shared::Plan) {
    crate::shared::set_plan(&app, &plan);
}

/// A window that can be waited on, with the process behind it.
#[derive(serde::Serialize)]
pub struct JobWindow {
    pid: u32,
    exe: String,
    name: String,
    title: String,
}

/// Windows to choose from when waiting for something to finish, one per process.
#[tauri::command]
pub async fn job_windows() -> Vec<JobWindow> {
    tauri::async_runtime::spawn_blocking(|| {
        multitasker_activity::process_windows()
            .into_iter()
            .map(|w| JobWindow { pid: w.pid, exe: w.exe, name: w.name, title: w.title })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// What tasks are waiting on, running or done.
#[tauri::command]
pub fn jobs() -> Vec<crate::jobs::Job> {
    crate::jobs::list()
}

/// Starts waiting on a program or a file for a task.
#[tauri::command]
pub fn wait_for(app: AppHandle, task_id: TaskId, task_title: String, label: String, what: crate::jobs::What) -> crate::jobs::Job {
    crate::jobs::add(&app, task_id, task_title, label, what)
}

#[tauri::command]
pub fn forget_job(app: AppHandle, id: String) {
    crate::jobs::remove(&app, &id);
}

/// Goes back to what a finished job was for: brings its browser tab up, and starts its task again if it was paused.
#[tauri::command]
pub async fn back_to_job(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let Some(job) = crate::jobs::go_back(&id) else { return Ok(()) };
    crate::jobs::remove(&app, &id);
    if job.task_id.is_empty() {
        return Ok(());
    }
    let running = task_views(&state).await?.into_iter().any(|t| t.task.id == job.task_id && t.in_progress);
    if !running {
        set_in_progress(app, state, job.task_id, true).await?;
    }
    Ok(())
}

/// Whether Windhawk is on this computer: its mods load into every program unless told not to, and that can keep the
/// browser's relay from starting.
#[tauri::command]
pub fn windhawk_present() -> bool {
    let installed = ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|var| std::env::var_os(var))
        .any(|dir| std::path::Path::new(&dir).join("Windhawk").join("windhawk.exe").is_file());
    installed || multitasker_activity::is_process_running("windhawk.exe").unwrap_or(false)
}

/// The browser relay started but never connected, and what had loaded its own code into it.
#[tauri::command]
pub fn relay_problem() -> Option<crate::relay_watch::RelayProblem> {
    crate::relay_watch::problem()
}

#[tauri::command]
pub fn dismiss_relay_problem(app: AppHandle) {
    crate::relay_watch::dismiss(&app);
}

/// Corrects a paused task's time spent to `total_secs`: the task list's own total, and the stretches kept here for the
/// Time view, newest first, so each day shows what was really worked.
#[tauri::command]
pub async fn correct_worked(app: AppHandle, state: State<'_, AppState>, task_id: TaskId, total_secs: i64) -> CmdResult<()> {
    let total = total_secs.clamp(0, 10 * 365 * 86_400);
    let task = task_views(&state)
        .await?
        .into_iter()
        .find(|t| t.task.id == task_id)
        .ok_or("That task no longer exists.")?;
    if task.in_progress {
        return Err("Pause the task before correcting its time.".into());
    }
    let provider = state.provider();
    let key = provider.key();
    let change = total - task.worked_secs;

    if change < 0 {
        state.state.trim_work(key, &task_id, -change).await.map_err(err)?;
    } else if change > 0 {
        let now = Utc::now();
        let session = WorkSession {
            id: uuid::Uuid::new_v4().to_string(),
            provider: key.to_owned(),
            task_id: task_id.clone(),
            device: state.device.id.clone(),
            started_at: now - chrono::Duration::seconds(change),
            ended_at: now,
            in_list: provider.extras().is_some(),
        };
        state.state.log_work(session).await.map_err(err)?;
    }

    // A list that keeps its own total gets what's left once the time kept only here is counted.
    if let Some(store) = provider.extras() {
        let here = state.state.worked(key).await.map_err(err)?.remove(&task_id).unwrap_or(0);
        store.set_worked(&task_id, (total - here).max(0)).await.map_err(err)?;
    }
    changed(&app);
    Ok(())
}
