use std::collections::HashMap;

use chrono::{DateTime, Utc};
use multitasker_core::{
    Attachment, AttachmentKind, NewAttachment, NewTask, Task, TaskChanges, TaskId, TaskStatus, TaskView, WorkSession,
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
                    worked_secs: worked.remove(&task.id).unwrap_or(0),
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
            TaskView {
                task,
                in_progress: started_at.is_some(),
                started_at,
                in_progress_on,
                started_on_name: None,
                attachments,
                deliverables,
                worked_secs,
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
        changed(&app);
        Ok(())
    } else {
        stop_task(&app, &state, &id, Utc::now()).await
    }
}

/// Pauses a task, counting the work on it up to `ended_at`, such as when the person left rather than now.
pub async fn stop_task(app: &AppHandle, state: &AppState, id: &TaskId, ended_at: DateTime<Utc>) -> CmdResult<()> {
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
    if let Some(started_at) = began.filter(|at| *at < ended_at) {
        let session = WorkSession {
            id: uuid::Uuid::new_v4().to_string(),
            provider: key.to_owned(),
            task_id: id.clone(),
            device: state.device.id.clone(),
            started_at,
            ended_at,
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
    TaskSourceView {
        source: settings::load(&state.data_dir).tasks,
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
