use std::collections::HashMap;

use multitasker_core::{NewTask, Task, TaskChanges, TaskId, TaskStatus, TaskView};
use tauri::{AppHandle, Emitter, State};

use crate::{windows, AppState};

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
    let tasks = state.provider.list().await.map_err(err)?;
    let key = state.provider.key();

    // Earliest start per task across every device: "in progress since" means since anyone picked it up.
    let mut started = HashMap::new();
    for entry in state.state.focus().await.map_err(err)?.into_iter().filter(|e| e.provider == key) {
        let at = started.entry(entry.task_id).or_insert(entry.started_at);
        *at = (*at).min(entry.started_at);
    }

    Ok(tasks
        .into_iter()
        .map(|task| {
            let started_at = started.get(&task.id).copied();
            TaskView { task, in_progress: started_at.is_some(), started_at }
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
    state.state.stop(state.provider.key(), &id).await.map_err(err)?;
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
