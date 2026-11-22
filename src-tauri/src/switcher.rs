use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// The quick switcher's window.
pub const SWITCHER: &str = "switcher";
/// Tells the switcher it was just opened, so it starts fresh.
const OPENED: &str = "switcher-opened";

/// The hotkeys Settings offers, in the order tried when the default is taken by another program.
pub const CHOICES: &[&str] = &["Ctrl+Alt+Space", "Ctrl+Shift+Space", "Ctrl+Alt+K", "Ctrl+Shift+K"];

/// The hotkey in use, kept so it can be let go while something is full screen and taken back after.
static HOTKEY: Mutex<Option<String>> = Mutex::new(None);
/// Whether the hotkey is let go right now because something is full screen.
static RELEASED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Lets go of the hotkey (`false`) so a full-screen program gets those keys, or takes it back (`true`).
pub fn hold_hotkey(app: &AppHandle, hold: bool) {
    use std::sync::atomic::Ordering;
    if RELEASED.load(Ordering::Relaxed) != hold {
        return;
    }
    let shortcuts = app.global_shortcut();
    if hold {
        let current = HOTKEY.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(key) = current {
            let _ = shortcuts.register(key.as_str());
        }
    } else {
        let _ = shortcuts.unregister_all();
    }
    RELEASED.store(!hold, Ordering::Relaxed);
}

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
    *HOTKEY.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let Some(shortcut) = shortcut else { return Ok(()) };
    shortcuts.register(shortcut).map_err(|_| format!("Another program already uses {shortcut}, so pick a different one."))?;
    *HOTKEY.lock().unwrap_or_else(|e| e.into_inner()) = Some(shortcut.to_owned());
    // A hotkey chosen while something is full screen waits until it isn't.
    if RELEASED.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = shortcuts.unregister_all();
    }
    Ok(())
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
