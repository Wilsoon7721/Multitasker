use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::detection::TrayToggle;
use crate::{settings, windows, AppState};

pub fn build(app: &AppHandle, detection: bool) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open MultiTasker", true, None::<&str>)?;
    let widget = MenuItem::with_id(app, "widget", "Show / hide widget", true, None::<&str>)?;
    let detect = CheckMenuItem::with_id(app, "detect", "Detect what I'm working on", true, detection, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = MenuBuilder::new(app).items(&[&open, &widget, &detect, &separator, &quit]).build()?;
    app.manage(TrayToggle(detect));

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("MultiTasker")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let _ = match event.id().as_ref() {
                "open" => windows::show_main(app),
                "widget" => windows::toggle_widget(app).map(|_| ()),
                "detect" => {
                    toggle_detection(app);
                    Ok(())
                }
                "quit" => {
                    app.exit(0);
                    Ok(())
                }
                _ => Ok(()),
            };
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let _ = windows::show_main(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }

    tray.build(app)?;
    Ok(())
}

// The menu item ticks itself on click; follow it and remember the choice.
fn toggle_detection(app: &AppHandle) {
    let Some(enabled) = app.try_state::<TrayToggle>().and_then(|t| t.0.is_checked().ok()) else { return };
    let state = app.state::<AppState>();
    let mut current = settings::load(&state.data_dir);
    current.detection = enabled;
    let _ = settings::save(&state.data_dir, &current);
    state.detector.set_enabled(app, enabled);
}
