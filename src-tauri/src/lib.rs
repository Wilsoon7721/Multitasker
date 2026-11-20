mod cloud;
mod commands;
mod detection;
mod device;
mod settings;
mod syncing;
mod tray;
mod windows;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use multitasker_core::{StateStore, TaskProvider};
use multitasker_provider_sqlite::SqliteDb;
use tauri::{Manager, WindowEvent};

/// Where tasks come from, where MultiTasker's own state lives, and which computer this is.
pub struct AppState {
    pub provider: Arc<dyn TaskProvider>,
    pub state: Arc<dyn StateStore>,
    pub device: device::Device,
    pub detector: Arc<detection::Detector>,
    pub syncer: Arc<syncing::Syncer>,
    /// The local database itself, for sync; `provider` and `state` are the same object behind their traits.
    pub db: Arc<SqliteDb>,
    pub data_dir: PathBuf,
    /// Set to stop waiting for a browser sign-in.
    pub sign_in_cancel: Arc<AtomicBool>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let db = Arc::new(SqliteDb::open(data_dir.join("multitasker.db"))?);
            let prefs = settings::load(&data_dir);
            let detector = detection::Detector::new(prefs.detection);
            let syncer = syncing::Syncer::new(prefs.sync.clone());
            app.manage(AppState {
                provider: db.clone(),
                state: db.clone(),
                device: device::load_or_create(&data_dir)?,
                detector: detector.clone(),
                syncer: syncer.clone(),
                db,
                data_dir,
                sign_in_cancel: Arc::new(AtomicBool::new(false)),
            });

            tray::build(app.handle(), prefs.detection)?;
            windows::set_widget_visible(app.handle(), true)?;
            detection::start(app.handle(), detector);
            syncing::start(app.handle(), syncer);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window only hides it; the app lives on in the tray with the widget.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == windows::MAIN {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_tasks,
            commands::create_task,
            commands::update_task,
            commands::delete_task,
            commands::set_in_progress,
            commands::add_attachments,
            commands::remove_attachment,
            commands::open_attachment,
            commands::reveal_attachment,
            commands::this_device,
            commands::open_main,
            commands::widget_visible,
            commands::set_widget_visible,
            commands::running_apps,
            commands::activity,
            commands::dismiss_suggestion,
            commands::set_detection,
            commands::detection_enabled,
            commands::sync_status,
            commands::set_sync_target,
            commands::sync_now,
            commands::sync_folder_suggestions,
            commands::sign_in_cloud,
            commands::cancel_sign_in,
            commands::built_in_clients,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MultiTasker");
}
