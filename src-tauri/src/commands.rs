use std::collections::HashMap;

use multitasker_core::{Attachment, AttachmentKind, NewAttachment, NewTask, Task, TaskChanges, TaskId, TaskStatus, TaskView};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

use crate::detection::{self, Snapshot};
use crate::{device::Device, settings, windows, AppState};

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

/// Every task with MultiTasker's own state joined in.
pub async fn task_views(state: &AppState) -> CmdResult<Vec<TaskView>> {
    let tasks = state.provider.list().await.map_err(err)?;
    let key = state.provider.key();

    // Earliest start per task across every device: "in progress since" means since anyone picked it up.
    let mut started = HashMap::new();
    for entry in state.state.focus().await.map_err(err)?.into_iter().filter(|e| e.provider == key) {
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
            TaskView { task, in_progress: started_at.is_some(), started_at, attachments }
        })
        .collect())
}

#[tauri::command]
pub async fn create_task(app: AppHandle, state: State<'_, AppState>, task: NewTask) -> CmdResult<Task> {
    let task = state.provider.create(task).await.map_err(err)?;
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
    let task = state.provider.update(&id, changes).await.map_err(err)?;
    // A finished task is no longer in progress anywhere.
    if task.status == TaskStatus::Done {
        state.state.stop(state.provider.key(), &id).await.map_err(err)?;
    }
    changed(&app);
    Ok(task)
}

#[tauri::command]
pub async fn delete_task(app: AppHandle, state: State<'_, AppState>, id: TaskId) -> CmdResult<()> {
    state.provider.delete(&id).await.map_err(err)?;
    state.state.forget(state.provider.key(), &id).await.map_err(err)?;
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
    let key = state.provider.key();
    if in_progress {
        // Starting a finished task reopens it.
        let task = state.provider.list().await.map_err(err)?.into_iter().find(|t| t.id == id);
        match task {
            None => return Err(format!("task {id} was not found")),
            Some(t) if t.status == TaskStatus::Done => {
                let reopen = TaskChanges { status: Some(TaskStatus::Todo), ..Default::default() };
                state.provider.update(&id, reopen).await.map_err(err)?;
            }
            Some(_) => {}
        }
        state.state.start(key, &id, &state.device.id).await.map_err(err)?;
    } else {
        state.state.stop(key, &id).await.map_err(err)?;
    }
    changed(&app);
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
    let key = state.provider.key();
    if !state.provider.list().await.map_err(err)?.iter().any(|t| t.id == id) {
        return Err(format!("task {id} was not found"));
    }

    let (mut added, mut problems) = (Vec::new(), Vec::new());
    for item in items {
        match Attachment::new(key, &id, &state.device.id, item) {
            Ok(a) => added.push(state.state.add_attachment(a).await.map_err(err)?),
            Err(e) => problems.push(e.to_string()),
        }
    }

    if !added.is_empty() {
        changed(&app);
    }
    if problems.is_empty() {
        Ok(added)
    } else {
        Err(problems.join(" "))
    }
}

#[tauri::command]
pub async fn remove_attachment(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.state.remove_attachment(&id).await.map_err(err)?;
    changed(&app);
    Ok(())
}

async fn find_attachment(state: &AppState, id: &str) -> CmdResult<Attachment> {
    state
        .state
        .attachments()
        .await
        .map_err(err)?
        .into_iter()
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
    let mut current = settings::load(&state.data_dir);
    current.detection = enabled;
    settings::save(&state.data_dir, &current).map_err(err)?;
    state.detector.set_enabled(&app, enabled);
    detection::sync_tray(&app, enabled);
    Ok(())
}
