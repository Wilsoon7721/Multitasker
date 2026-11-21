use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// The quick switcher's window.
pub const SWITCHER: &str = "switcher";
/// Tells the switcher it was just opened, so it starts fresh.
const OPENED: &str = "switcher-opened";

/// Handles the hotkey wherever MultiTasker is, showing or hiding the switcher.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle(app);
            }
        })
        .build()
}

/// Listens for the hotkey; `None` turns it off. Says so when another program already uses it.
pub fn set_shortcut(app: &AppHandle, shortcut: Option<&str>) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    shortcuts.unregister_all().map_err(|e| e.to_string())?;
    let Some(shortcut) = shortcut else { return Ok(()) };
    shortcuts
        .register(shortcut)
        .map_err(|e| format!("{shortcut} can't be used; another program may have it already ({e})."))
}

pub fn toggle(app: &AppHandle) {
    let Some(window) = app.get_webview_window(SWITCHER) else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let _ = window.center();
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit_to(SWITCHER, OPENED, ());
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SWITCHER) {
        let _ = window.hide();
    }
}
