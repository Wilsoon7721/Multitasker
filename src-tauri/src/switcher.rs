use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// The quick switcher's window.
pub const SWITCHER: &str = "switcher";
/// Tells the switcher it was just opened, so it starts fresh.
const OPENED: &str = "switcher-opened";

/// The hotkeys Settings offers, in the order tried when the default is taken by another program.
pub const CHOICES: &[&str] = &["Ctrl+Alt+Space", "Ctrl+Shift+Space", "Ctrl+Alt+K", "Ctrl+Shift+K"];

/// Why the chosen hotkey isn't working, for Settings to say.
static PROBLEM: Mutex<Option<String>> = Mutex::new(None);

pub fn problem() -> Option<String> {
    PROBLEM.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_problem(problem: Option<String>) {
    *PROBLEM.lock().unwrap_or_else(|e| e.into_inner()) = problem;
}

/// Listens for the saved hotkey at startup. When it's the default and another program has it, the next free choice
/// is used instead and returned, to be saved.
pub fn start(app: &AppHandle, saved: Option<&str>) -> Option<String> {
    match set_shortcut(app, saved) {
        Ok(()) => None,
        Err(e) if saved == CHOICES.first().copied() => {
            let free = CHOICES[1..].iter().find(|c| set_shortcut(app, Some(c)).is_ok());
            if free.is_none() {
                set_problem(Some(e));
            }
            free.map(|c| c.to_string())
        }
        Err(e) => {
            set_problem(Some(e));
            None
        }
    }
}

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
    set_problem(None);
    let Some(shortcut) = shortcut else { return Ok(()) };
    shortcuts.register(shortcut).map_err(|_| format!("Another program already uses {shortcut}, so pick a different one."))
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
