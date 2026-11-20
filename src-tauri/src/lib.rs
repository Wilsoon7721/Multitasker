mod commands;
mod device;
mod tray;
mod windows;

use std::sync::Arc;

use multitasker_core::{StateStore, TaskProvider};
use multitasker_provider_sqlite::SqliteDb;
use tauri::{Manager, WindowEvent};

/// Where tasks come from, where MultiTasker's own state lives, and which computer this is.
pub struct AppState {
    pub provider: Arc<dyn TaskProvider>,
    pub state: Arc<dyn StateStore>,
    pub device: device::Device,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let db = Arc::new(SqliteDb::open(data_dir.join("multitasker.db"))?);
            app.manage(AppState { provider: db.clone(), state: db, device: device::load_or_create(&data_dir)? });

            tray::build(app.handle())?;
            windows::set_widget_visible(app.handle(), true)?;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running MultiTasker");
}
