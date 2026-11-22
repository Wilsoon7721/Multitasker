mod away;
mod bridge;
mod checks;
mod cloud;
mod commands;
mod detection;
mod device;
mod here;
mod jobs;
mod on_top;
mod projects;
mod reminders;
mod settings;
mod shared;
mod switcher;
mod syncing;
mod tokens;
mod tray;
mod updates;
mod wilsoon;
mod windows;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use multitasker_core::{StateStore, TaskProvider};
use multitasker_provider_sqlite::SqliteDb;
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_deep_link::DeepLinkExt;

/// Where tasks come from, where MultiTasker's own state lives, and which computer this is.
pub struct AppState {
    /// Where tasks come from right now; switched from Settings.
    provider: RwLock<Arc<dyn TaskProvider>>,
    pub state: Arc<dyn StateStore>,
    pub device: device::Device,
    pub detector: Arc<detection::Detector>,
    pub syncer: Arc<syncing::Syncer>,
    /// The local database itself, for sync; `provider` and `state` are the same object behind their traits.
    pub db: Arc<SqliteDb>,
    pub data_dir: PathBuf,
    /// Set to stop waiting for a browser sign-in.
    pub sign_in_cancel: Arc<AtomicBool>,
    /// A WilsoonID sign-in waiting for the browser to come back through the app link.
    pub wilsoon_sign_in: wilsoon::SignInSlot,
    /// Copies of work handed in for tasks, kept on this computer.
    pub deliverables: Arc<multitasker_deliverables::Store>,
    /// Dev servers and other project commands started from tasks.
    pub projects: Arc<projects::Projects>,
}

impl AppState {
    pub fn provider(&self) -> Arc<dyn TaskProvider> {
        self.provider.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_provider(&self, provider: Arc<dyn TaskProvider>) {
        *self.provider.write().unwrap_or_else(|e| e.into_inner()) = provider;
    }
}

/// The task source settings ask for, falling back to local tasks if its sign-in or token is gone.
fn initial_provider(source: &settings::TaskSource, db: Arc<SqliteDb>) -> Arc<dyn TaskProvider> {
    match source {
        settings::TaskSource::JavaDashboard { .. } => match wilsoon::tokens() {
            Ok(tokens) => Arc::new(wilsoon::provider(tokens)),
            Err(_) => db,
        },
        settings::TaskSource::Todoist { .. } => match tokens::todoist() {
            Some(todoist) => Arc::new(todoist),
            None => db,
        },
        settings::TaskSource::Notion { database, .. } => match tokens::notion(database) {
            Some(notion) => Arc::new(notion),
            None => db,
        },
        settings::TaskSource::Local => db,
    }
}

pub fn run() {
    // Chrome or Edge started this copy to carry messages for the extension; it never opens a window.
    if bridge::is_relay() {
        bridge::relay();
        return;
    }
    tauri::Builder::default()
        // Another copy started by an app link (a sign-in coming back) hands its URL over here and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = windows::show_main(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![updates::AUTOSTART_ARG]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(switcher::plugin())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            move_old_data_dir(&data_dir);
            std::fs::create_dir_all(&data_dir)?;
            shared::ensure(&data_dir);

            let db = Arc::new(SqliteDb::open(data_dir.join("multitasker.db"))?);
            let deliverables = Arc::new(multitasker_deliverables::Store::open(data_dir.join("deliverables"))?);
            let prefs = settings::load(&data_dir);
            let device = device::load_or_create(&data_dir)?;
            let detector = detection::Detector::new(prefs.detection, prefs.auto_switch, prefs.auto_start_after_secs, device.id.clone());
            let syncer = syncing::Syncer::new(prefs.sync.clone());
            app.manage(AppState {
                provider: RwLock::new(initial_provider(&prefs.tasks, db.clone())),
                state: db.clone(),
                device,
                detector: detector.clone(),
                syncer: syncer.clone(),
                db,
                data_dir,
                sign_in_cancel: Arc::new(AtomicBool::new(false)),
                wilsoon_sign_in: wilsoon::SignInSlot::default(),
                deliverables,
                projects: Arc::default(),
            });

            // The windows open only now, so their first questions never arrive before the state they ask about.
            for config in app.config().app.windows.clone() {
                tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
            }

            // Installers register the app link; development builds register it here so sign-in works there too.
            #[cfg(any(windows, target_os = "linux"))]
            let _ = app.deep_link().register_all();
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                let state = handle.state::<AppState>();
                for url in event.urls() {
                    state.wilsoon_sign_in.deliver(url.as_str());
                }
            });
            start_remote_refresh(app.handle());
            bridge::serve(app.handle());
            if let Err(e) = bridge::register(&app.state::<AppState>().data_dir) {
                eprintln!("{e}");
            }

            tray::build(app.handle(), prefs.detection)?;
            windows::set_widget_visible(app.handle(), true)?;
            detection::start(app.handle(), detector);
            syncing::start(app.handle(), syncer);
            shared::start(app.handle());
            jobs::start(app.handle());
            reminders::start(app.handle());
            away::start(app.handle());
            on_top::start(app.handle());
            if let Err(e) = updates::apply_autostart(app.handle(), prefs.start_with_windows) {
                eprintln!("couldn't set starting with Windows: {e}");
            }
            updates::start(app.handle());
            if let Some(free) = switcher::start(app.handle(), prefs.switcher_shortcut.as_deref()) {
                // Only this computer has the usual hotkey taken, so the stand-in isn't shared.
                let _ = settings::update_quiet(&app.state::<AppState>().data_dir, |s| s.switcher_shortcut = Some(free));
            }
            // Started by Windows at sign-in, it opens quietly to the tray and widget.
            if !updates::started_at_sign_in() {
                windows::show_main(app.handle())?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // The switcher is only for a moment; clicking anywhere else puts it away.
            if let WindowEvent::Focused(false) = event {
                if window.label() == switcher::SWITCHER {
                    let _ = window.hide();
                }
            }
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
            commands::list_deliverables,
            commands::hand_in,
            commands::open_deliverable,
            commands::export_deliverable,
            commands::remove_deliverable,
            commands::this_device,
            commands::open_main,
            commands::widget_visible,
            commands::set_widget_visible,
            commands::running_apps,
            commands::activity,
            commands::dismiss_suggestion,
            commands::set_detection,
            commands::detection_enabled,
            commands::dismiss_paused,
            commands::add_check,
            commands::set_check,
            commands::remove_check,
            commands::task_openable,
            commands::project_commands,
            commands::project_runs,
            commands::run_project_command,
            commands::stop_project_command,
            commands::open_project_url,
            commands::set_project_autorun,
            commands::project_autoruns,
            commands::work_sessions,
            commands::hide_switcher,
            commands::switcher_shortcut,
            commands::set_switcher_shortcut,
            commands::switcher_problem,
            commands::update_info,
            commands::check_for_updates,
            commands::install_update,
            commands::start_with_windows,
            commands::set_start_with_windows,
            commands::check_hand_in,
            commands::check_link,
            commands::hand_in_link,
            commands::widget_on_top,
            commands::set_widget_on_top,
            commands::accept_link_offer,
            commands::refuse_link_offer,
            commands::set_left_off,
            commands::open_task_things,
            commands::switch_task,
            commands::undo_switch,
            commands::dismiss_switch,
            commands::auto_switch,
            commands::set_auto_switch,
            commands::auto_start_after,
            commands::set_auto_start_after,
            commands::pause_after_away,
            commands::set_pause_after_away,
            commands::sync_status,
            commands::set_sync_target,
            commands::sync_now,
            commands::sync_folder_suggestions,
            commands::sign_in_cloud,
            commands::cancel_sign_in,
            commands::built_in_clients,
            commands::reminder_settings,
            commands::set_reminder_settings,
            commands::task_source,
            commands::widget_opacity,
            commands::set_widget_opacity,
            commands::settings_sync,
            commands::day_plan,
            commands::set_day_plan,
            commands::job_windows,
            commands::jobs,
            commands::wait_for,
            commands::forget_job,
            commands::back_to_job,
            commands::sign_in_wilsoon,
            commands::use_local_tasks,
            commands::connect_todoist,
            commands::notion_databases,
            commands::connect_notion,
        ])
        .build(tauri::generate_context!())
        .expect("error while building MultiTasker")
        .run(|app, event| {
            // Dev servers started from tasks don't outlive MultiTasker.
            if let tauri::RunEvent::Exit = event {
                app.state::<AppState>().projects.stop_all();
            }
        });
}

/// Early builds used the identifier `io.github.wilsoon7721.multitasker`; carry their data over once.
fn move_old_data_dir(data_dir: &std::path::Path) {
    let Some(old) = data_dir.parent().map(|p| p.join("io.github.wilsoon7721.multitasker")) else { return };
    if !data_dir.exists() && old.is_dir() {
        let _ = std::fs::rename(old, data_dir);
    }
}

/// Tasks edited in JavaDashboard itself only show up when asked for, so ask every minute while it's the source.
fn start_remote_refresh(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("remote-refresh".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let state = app.state::<AppState>();
            if state.provider().key() != multitasker_provider_sqlite::PROVIDER_KEY {
                let _ = tauri::async_runtime::block_on(commands::catch_up_work(&state));
                let _ = app.emit(commands::TASKS_CHANGED, ());
            }
        })
        .expect("failed to start the refresh thread");
}
