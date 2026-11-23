use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use multitasker_activity::LoadedModule;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::AppState;

/// Tells the main window the relay got stuck, or that it's working again.
pub const RELAY_PROBLEM: &str = "relay-problem";

/// The copy of MultiTasker the browser runs to carry the extension's messages.
const RELAY_EXE: &str = "multitasker-host.exe";
/// A relay connects within a couple of seconds; one running this long without connecting is stuck.
const STUCK_AFTER: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(10);
/// Stuck relays are stopped so the browser starts fresh ones, but not forever if every one gets stuck.
const MAX_RESTARTS: u32 = 3;

/// A relay that started but never got going, and the programs that had put their own code into it.
#[derive(Debug, Clone, Serialize)]
pub struct RelayProblem {
    pub since: DateTime<Utc>,
    pub suspects: Vec<Suspect>,
    /// Stuck relays stopped so far, so the browser could start fresh ones.
    pub restarts: u32,
    /// They kept getting stuck, so MultiTasker stopped restarting them.
    pub gave_up: bool,
}

/// A program whose code was loaded into the relay, with the files it put there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suspect {
    pub name: String,
    pub files: Vec<String>,
}

static PROBLEM: Mutex<Option<RelayProblem>> = Mutex::new(None);

pub fn problem() -> Option<RelayProblem> {
    PROBLEM.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn dismiss(app: &AppHandle) {
    *PROBLEM.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let _ = app.emit(RELAY_PROBLEM, ());
}

/// The programs behind the code loaded into a relay: everything that isn't Windows' own or the relay itself, grouped
/// by the product it says it belongs to, or else the folder it was installed in.
pub fn suspects(modules: &[LoadedModule], windows_dir: &str) -> Vec<Suspect> {
    let windows_dir = windows_dir.trim_end_matches('\\').to_lowercase();
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // The first module is the program itself.
    for module in modules.iter().skip(1) {
        let lower = module.path.to_lowercase();
        if !windows_dir.is_empty() && lower.starts_with(&format!("{windows_dir}\\")) {
            continue;
        }
        let file = module.path.rsplit(['\\', '/']).next().unwrap_or(&module.path).to_owned();
        let name = module
            .product
            .clone()
            .or_else(|| module.company.clone())
            .or_else(|| install_folder(&module.path))
            .unwrap_or_else(|| file.clone());
        found.entry(name).or_default().push(file);
    }
    // A program's files may name it by product in some and only by folder in others; those are joined up.
    let mut merged: Vec<Suspect> = Vec::new();
    for (name, files) in found {
        match merged.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&name)) {
            Some(s) => s.files.extend(files),
            None => merged.push(Suspect { name, files }),
        }
    }
    merged
}

/// The folder a file was installed under: the first one inside Program Files, ProgramData or AppData.
fn install_folder(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
    let roots = ["program files", "program files (x86)", "programdata", "local", "roaming"];
    let at = parts.iter().position(|p| roots.contains(&p.to_lowercase().as_str()))?;
    parts.get(at + 1).filter(|_| at + 2 < parts.len()).map(|p| p.to_string())
}

fn stop(pid: u32) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

/// One look: relays running a while that never connected are stuck; say what was loaded into them, and stop them so
/// the browser can start fresh ones.
fn check(app: &AppHandle) {
    let (connected, unnamed) = crate::bridge::connected_relays();
    let now = SystemTime::now();
    let unaccounted: Vec<_> = multitasker_activity::processes_named(RELAY_EXE)
        .into_iter()
        .filter(|p| !connected.contains(&p.pid))
        .collect();
    // Older relays connect without saying which they are; that many unaccounted ones may simply be them.
    let stuck: Vec<_> = if unaccounted.len() <= unnamed {
        Vec::new()
    } else {
        unaccounted.into_iter().filter(|p| now.duration_since(p.started).unwrap_or_default() >= STUCK_AFTER).collect()
    };

    let mut problem = PROBLEM.lock().unwrap_or_else(|e| e.into_inner());
    if stuck.is_empty() {
        // Working again once a relay has connected.
        if problem.is_some() && (!connected.is_empty() || unnamed > 0) {
            *problem = None;
            drop(problem);
            let _ = app.emit(RELAY_PROBLEM, ());
        }
        return;
    }

    let windows_dir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let fresh = problem.is_none();
    let current = problem.get_or_insert_with(|| RelayProblem { since: Utc::now(), suspects: Vec::new(), restarts: 0, gave_up: false });
    for relay in &stuck {
        if let Some(modules) = multitasker_activity::process_modules(relay.pid) {
            for suspect in suspects(&modules, &windows_dir) {
                if !current.suspects.iter().any(|s| s.name == suspect.name) {
                    current.suspects.push(suspect);
                }
            }
        }
        eprintln!("browser relay {} is stuck; loaded into it: {:?}", relay.pid, current.suspects);
        if current.restarts < MAX_RESTARTS {
            stop(relay.pid);
            current.restarts += 1;
        } else {
            current.gave_up = true;
        }
    }
    let names: Vec<String> = current.suspects.iter().map(|s| s.name.clone()).collect();
    drop(problem);

    // A relay that ran an outdated copy is replaced with this version's before the browser starts the next one.
    let _ = crate::bridge::register(&app.state::<AppState>().data_dir);
    if fresh {
        let body = if names.is_empty() {
            "Open MultiTasker to see what to do.".to_owned()
        } else {
            format!("{} may be getting in its way. Open MultiTasker to see what to do.", names.join(", "))
        };
        let _ = app.notification().builder().title("The browser extension can't reach MultiTasker").body(body).show();
    }
    let _ = app.emit(RELAY_PROBLEM, ());
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("relay-watch".into())
        .spawn(move || loop {
            std::thread::sleep(CHECK_EVERY);
            check(&app);
        })
        .expect("failed to start the relay watch thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(path: &str, product: Option<&str>) -> LoadedModule {
        LoadedModule { path: path.into(), product: product.map(Into::into), company: None, description: None }
    }

    #[test]
    fn names_the_programs_that_loaded_code_into_the_relay() {
        let modules = [
            module(r"C:\Users\me\AppData\Roaming\dev.wilsoon.multitasker\browser-host\multitasker-host.exe", Some("MultiTasker")),
            module(r"C:\Windows\System32\ntdll.dll", Some("Microsoft® Windows® Operating System")),
            module(r"C:\Windows\SYSTEM32\kernel32.dll", None),
            module(r"C:\Program Files\Windhawk\windhawk.dll", Some("Windhawk")),
            module(r"C:\ProgramData\Windhawk\Engine\Mods\64\explorer-details_1.5.1.dll", None),
            module(r"C:\ProgramData\Windhawk\Engine\Mods\64\libc++.whl", None),
            module(r"C:\Program Files\Some Overlay\hook64.dll", None),
        ];
        let found = suspects(&modules, r"C:\Windows");
        assert_eq!(found.len(), 2, "{found:?}");
        let windhawk = found.iter().find(|s| s.name == "Windhawk").unwrap();
        assert_eq!(windhawk.files, ["windhawk.dll", "explorer-details_1.5.1.dll", "libc++.whl"]);
        assert_eq!(found.iter().find(|s| s.name == "Some Overlay").unwrap().files, ["hook64.dll"]);
    }

    #[test]
    fn finds_the_folder_a_file_was_installed_in() {
        assert_eq!(install_folder(r"C:\Program Files (x86)\Vendor\x\hook.dll").as_deref(), Some("Vendor"));
        assert_eq!(install_folder(r"C:\Users\me\AppData\Local\Tool\hook.dll").as_deref(), Some("Tool"));
        assert_eq!(install_folder(r"D:\loose\hook.dll"), None);
    }
}
