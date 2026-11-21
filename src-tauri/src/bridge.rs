use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use multitasker_core::{NewAttachment, NewTask, TaskStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::commands;
use crate::AppState;

/// The browser extension's host name, as Chrome and Edge know it.
pub const HOST: &str = "dev.wilsoon.multitasker";
/// The extension's id, fixed by the public key in `extension/manifest.json`.
pub const EXTENSION_ID: &str = "fldigbhidkmfhlgplcmgjmbhbgkdmebj";
/// Where the running app listens for the relay; only this computer can connect.
const PIPE: &str = r"\\.\pipe\dev.wilsoon.multitasker";
/// Chrome caps a message to the extension at 1 MB.
const MAX_REPLY: usize = 1024 * 1024;

/// Whether Chrome or Edge started this process to talk to the extension.
pub fn is_relay() -> bool {
    std::env::args().nth(1).is_some_and(|a| a.starts_with("chrome-extension://"))
}

/// A request from the extension; `id` comes back on the reply so it can match them up.
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    id: Value,
    #[serde(flatten)]
    request: Request,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    /// Open tasks, with which of them the given page is linked to.
    Tasks { url: Option<String> },
    /// Links a page to a task.
    Link { task_id: String, url: String, title: Option<String> },
    Unlink { attachment_id: String },
    Start { task_id: String },
    /// A new task with the page linked to it.
    Create { title: String, url: Option<String>, page_title: Option<String> },
    /// The tab in front changed; `None` when the browser lost focus.
    ActiveTab { url: Option<String> },
    ShowApp,
}

#[derive(Debug, Serialize)]
struct TaskItem {
    id: String,
    title: String,
    category: Option<String>,
    in_progress: bool,
    /// The link attachment for the asked-about page, if it's linked to this task.
    linked: Option<String>,
}

async fn handle(app: &AppHandle, request: Request) -> Result<Value, String> {
    let state = app.state::<AppState>();
    match request {
        Request::Tasks { url } => {
            let wanted = url.as_deref().map(multitasker_activity::normalize_url);
            let mut tasks: Vec<TaskItem> = commands::task_views(&state)
                .await?
                .into_iter()
                .filter(|t| t.task.status == TaskStatus::Todo)
                .map(|t| TaskItem {
                    linked: wanted.as_ref().and_then(|w| {
                        t.attachments
                            .iter()
                            .find(|a| a.kind == multitasker_core::AttachmentKind::Link && &multitasker_activity::normalize_url(&a.target) == w)
                            .map(|a| a.id.clone())
                    }),
                    id: t.task.id,
                    title: t.task.title,
                    category: t.task.category,
                    in_progress: t.in_progress,
                })
                .collect();
            tasks.sort_by_key(|t| (t.linked.is_none(), !t.in_progress));
            Ok(json!({ "tasks": tasks }))
        }
        Request::Link { task_id, url, title } => {
            let item = NewAttachment::Link { url, name: title.filter(|t| !t.trim().is_empty()) };
            let added = commands::add_attachments(app.clone(), state, task_id, vec![item]).await?;
            Ok(json!({ "attachment_id": added.first().map(|a| a.id.clone()) }))
        }
        Request::Unlink { attachment_id } => {
            commands::remove_attachment(app.clone(), state, attachment_id).await?;
            Ok(json!({}))
        }
        Request::Start { task_id } => {
            commands::set_in_progress(app.clone(), state, task_id, true).await?;
            Ok(json!({}))
        }
        Request::Create { title, url, page_title } => {
            let task = commands::create_task(app.clone(), app.state::<AppState>(), NewTask { title, ..Default::default() }).await?;
            let mut linked = None;
            if let Some(url) = url {
                let item = NewAttachment::Link { url, name: page_title.filter(|t| !t.trim().is_empty()) };
                linked = commands::attach(app, &state, task.id.clone(), vec![item]).await?.first().map(|a| a.id.clone());
            }
            let item = TaskItem { id: task.id, title: task.title, category: task.category, in_progress: false, linked };
            Ok(json!({ "task": item }))
        }
        Request::ActiveTab { url } => {
            let linked: Vec<Value> = state
                .detector
                .set_tab(url)
                .into_iter()
                .map(|(task_id, title)| json!({ "task_id": task_id, "title": title }))
                .collect();
            Ok(json!({ "linked": linked }))
        }
        Request::ShowApp => {
            crate::windows::show_main(app).map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
    }
}

/// Answers one line from the relay with one line back.
async fn answer(app: &AppHandle, line: &str) -> String {
    let reply = match serde_json::from_str::<Envelope>(line) {
        Ok(Envelope { id, request }) => match handle(app, request).await {
            Ok(mut body) => {
                body["id"] = id;
                body
            }
            Err(error) => json!({ "id": id, "error": error }),
        },
        Err(e) => {
            let id = serde_json::from_str::<Value>(line).ok().and_then(|v| v.get("id").cloned()).unwrap_or(Value::Null);
            json!({ "id": id, "error": format!("MultiTasker didn't understand that: {e}") })
        }
    };
    reply.to_string()
}

/// Listens for the relay for as long as the app runs.
#[cfg(windows)]
pub fn serve(app: &AppHandle) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        loop {
            let server = match ServerOptions::new().first_pipe_instance(first).reject_remote_clients(true).create(PIPE) {
                Ok(s) => s,
                // Another copy of MultiTasker already listens; the relay talks to that one.
                Err(_) if first => return,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            first = false;
            if server.connect().await.is_err() {
                continue;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let (read, mut write) = tokio::io::split(server);
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let reply = answer(&app, &line).await;
                    if write.write_all(format!("{reply}\n").as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
}

#[cfg(not(windows))]
pub fn serve(_app: &AppHandle) {}

/// Requests that are worth starting MultiTasker for; a tab switch alone isn't.
fn starts_app(message: &Value) -> bool {
    message.get("type").and_then(|t| t.as_str()) != Some("active_tab")
}

/// The relay: passes Chrome's length-prefixed messages to the running app and its replies back.
pub fn relay() {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut pipe: Option<(std::fs::File, BufReader<std::fs::File>)> = None;
    loop {
        let mut len = [0u8; 4];
        if stdin.read_exact(&mut len).is_err() {
            return;
        }
        let mut message = vec![0u8; u32::from_le_bytes(len) as usize];
        if stdin.read_exact(&mut message).is_err() {
            return;
        }
        let parsed: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
        let reply = match exchange(&mut pipe, &message, starts_app(&parsed)) {
            Ok(reply) if reply.len() <= MAX_REPLY => reply,
            Ok(_) => json!({ "id": parsed.get("id"), "error": "MultiTasker's answer was too big to pass on." }).to_string(),
            Err(error) => {
                pipe = None;
                json!({ "id": parsed.get("id"), "error": error, "offline": true }).to_string()
            }
        };
        let bytes = reply.as_bytes();
        if stdout.write_all(&(bytes.len() as u32).to_le_bytes()).and_then(|_| stdout.write_all(bytes)).and_then(|_| stdout.flush()).is_err() {
            return;
        }
    }
}

fn exchange(pipe: &mut Option<(std::fs::File, BufReader<std::fs::File>)>, message: &[u8], start: bool) -> Result<String, String> {
    if pipe.is_none() {
        *pipe = Some(connect(start)?);
    }
    let (write, read) = pipe.as_mut().expect("just connected");
    let sent = write.write_all(message).and_then(|_| write.write_all(b"\n")).and_then(|_| write.flush());
    let mut line = String::new();
    match sent.and_then(|_| read.read_line(&mut line)) {
        Ok(n) if n > 0 => Ok(line.trim_end().to_owned()),
        _ => {
            // The app restarted since the last message; one fresh connection is worth a try.
            *pipe = Some(connect(start)?);
            let (write, read) = pipe.as_mut().expect("just connected");
            write.write_all(message).and_then(|_| write.write_all(b"\n")).and_then(|_| write.flush()).map_err(|e| e.to_string())?;
            let mut line = String::new();
            read.read_line(&mut line).map_err(|e| e.to_string())?;
            Ok(line.trim_end().to_owned())
        }
    }
}

fn open_pipe() -> std::io::Result<(std::fs::File, BufReader<std::fs::File>)> {
    let file = std::fs::OpenOptions::new().read(true).write(true).open(PIPE)?;
    let reader = BufReader::new(file.try_clone()?);
    Ok((file, reader))
}

fn connect(start: bool) -> Result<(std::fs::File, BufReader<std::fs::File>), String> {
    if let Ok(pipe) = open_pipe() {
        return Ok(pipe);
    }
    if !start {
        return Err("MultiTasker isn't running.".into());
    }
    std::process::Command::new(app_exe()?).spawn().map_err(|e| format!("Couldn't start MultiTasker: {e}"))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(400));
        if let Ok(pipe) = open_pipe() {
            return Ok(pipe);
        }
    }
    Err("MultiTasker didn't start in time; try again in a moment.".into())
}

/// The relay runs from a copy beside this file, which names the app it starts.
const APP_PATH: &str = "app-path.txt";

/// The MultiTasker the relay starts: the one that set it up, or itself.
fn app_exe() -> Result<std::path::PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let named = me.parent().and_then(|dir| std::fs::read_to_string(dir.join(APP_PATH)).ok());
    Ok(named.map(|p| std::path::PathBuf::from(p.trim())).filter(|p| p.is_file()).unwrap_or(me))
}

/// Tells Chrome and Edge where the relay is, so the extension can reach this copy of MultiTasker.
pub fn register(data_dir: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // The browser keeps the relay running, so it runs from a copy and never locks the app's own file.
    let dir = data_dir.join("browser-host");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let host = dir.join("multitasker-host.exe");
    let stamp = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()));
    if stamp(&host).map(|(len, _)| len) != stamp(&exe).map(|(len, _)| len)
        || stamp(&host).and_then(|(_, t)| t) < stamp(&exe).and_then(|(_, t)| t)
    {
        // A copy still in use by a running relay keeps working; it's replaced next time.
        let _ = std::fs::copy(&exe, &host);
    }
    std::fs::write(dir.join(APP_PATH), exe.display().to_string()).map_err(|e| e.to_string())?;
    let path = if host.is_file() { host } else { exe };
    let manifest = json!({
        "name": HOST,
        "description": "MultiTasker",
        "path": path,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    });
    let path = data_dir.join("browser-host.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    for browser in [r"Software\Google\Chrome", r"Software\Microsoft\Edge", r"Software\BraveSoftware\Brave-Browser"] {
        registry::set_default(&format!(r"{browser}\NativeMessagingHosts\{HOST}"), &path.display().to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// Sets a key's default value under HKEY_CURRENT_USER, making the key if needed.
    pub fn set_default(key: &str, value: &str) -> Result<(), String> {
        let (key, value) = (wide(key), wide(value));
        let mut handle: HKEY = std::ptr::null_mut();
        unsafe {
            let made = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            );
            if made != ERROR_SUCCESS {
                return Err(format!("Couldn't register the browser extension's host ({made})."));
            }
            let set = RegSetValueExW(handle, std::ptr::null(), 0, REG_SZ, value.as_ptr() as *const u8, (value.len() * 2) as u32);
            RegCloseKey(handle);
            if set != ERROR_SUCCESS {
                return Err(format!("Couldn't register the browser extension's host ({set})."));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_extensions_requests() {
        let e: Envelope = serde_json::from_str(r#"{"id":7,"type":"link","task_id":"t1","url":"https://claude.ai/chat/1","title":"GS chat"}"#).unwrap();
        assert_eq!(e.id, json!(7));
        assert!(matches!(e.request, Request::Link { ref task_id, .. } if task_id == "t1"));
        let e: Envelope = serde_json::from_str(r#"{"type":"active_tab","url":null}"#).unwrap();
        assert!(matches!(e.request, Request::ActiveTab { url: None }));
    }

    #[test]
    fn only_deliberate_requests_start_the_app() {
        assert!(!starts_app(&json!({ "type": "active_tab", "url": "https://x" })));
        assert!(starts_app(&json!({ "type": "tasks" })));
    }
}
