use std::sync::RwLock;
use std::time::Duration;

use multitasker_activity::{foreground_is_fullscreen, foreground_window};
use tauri::{AppHandle, Manager};

use crate::settings::{self, OnTop};
use crate::windows::WIDGET;

/// How often the window in front is checked against the exceptions.
const CHECK_EVERY: Duration = Duration::from_millis(1000);

static PREFS: RwLock<Option<OnTop>> = RwLock::new(None);

fn prefs() -> OnTop {
    PREFS.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Takes new preferences; the next check applies them.
pub fn set(prefs: OnTop) {
    *PREFS.write().unwrap_or_else(|e| e.into_inner()) = Some(prefs);
}

/// Whether the widget should stay on top while `exe` is in front.
fn wanted(prefs: &OnTop, exe: Option<&str>, fullscreen: bool) -> bool {
    if !prefs.enabled {
        return false;
    }
    let excepted = exe.is_some_and(|exe| prefs.exceptions.iter().any(|e| e.exe.eq_ignore_ascii_case(exe)));
    !excepted && !(prefs.except_fullscreen && fullscreen)
}

/// Keeps the widget on top of other windows, except while one of the excepted programs is in front.
pub fn start(app: &AppHandle) {
    set(settings::load(&app.state::<crate::AppState>().data_dir).widget_on_top);
    let app = app.clone();
    std::thread::Builder::new()
        .name("on-top".into())
        .spawn(move || {
            let me = std::process::id();
            let mut applied: Option<bool> = None;
            loop {
                let front = foreground_window();
                // Clicking MultiTasker's own windows leaves things as they were.
                if front.as_ref().is_none_or(|w| w.pid != me) {
                    let exe = front.as_ref().map(|w| w.exe.as_str());
                    let on_top = wanted(&prefs(), exe, front.is_some() && foreground_is_fullscreen());
                    if applied != Some(on_top) {
                        if let Some(widget) = app.get_webview_window(WIDGET) {
                            if widget.set_always_on_top(on_top).is_ok() {
                                applied = Some(on_top);
                            }
                        }
                    }
                }
                std::thread::sleep(CHECK_EVERY);
            }
        })
        .expect("failed to start the on-top thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ExceptedApp;

    #[test]
    fn steps_aside_for_excepted_programs_and_full_screen() {
        let prefs = OnTop {
            enabled: true,
            except_fullscreen: true,
            exceptions: vec![ExceptedApp { exe: "POWERPNT.EXE".into(), name: "PowerPoint".into() }],
        };
        assert!(wanted(&prefs, Some("Code.exe"), false));
        assert!(!wanted(&prefs, Some("powerpnt.exe"), false), "names match whatever their case");
        assert!(!wanted(&prefs, Some("vlc.exe"), true), "full screen steps aside");
        assert!(wanted(&OnTop { except_fullscreen: false, ..prefs.clone() }, Some("vlc.exe"), true));
        assert!(!wanted(&OnTop { enabled: false, ..prefs }, Some("Code.exe"), false));
    }
}
