use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use multitasker_core::{NewAttachment, NewTask, TaskStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::commands;
use crate::AppState;

/// The browser extension's host name, as Chrome and Edge know it.
pub const HOST: &str = "dev.wilsoon.multitasker";
/// The extension's ids: the one the key in `extension/manifest.json` fixes for unpacked copies, then each store's own.
pub const EXTENSION_IDS: &[&str] = &["fldigbhidkmfhlgplcmgjmbhbgkdmebj"];
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
    ActiveTab {
        url: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
    ShowApp,
    /// An AI chat in a tab started writing a reply.
    AiStarted { tab_id: i64, site: String },
    /// It finished; `looking` says whether its tab was in front at the time.
    AiDone { tab_id: i64, looking: bool },
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
        Request::ActiveTab { url, title } => {
            let linked: Vec<Value> = state
                .detector
                .set_tab(url, title)
                .into_iter()
                .map(|(task_id, title)| json!({ "task_id": task_id, "title": title }))
                .collect();
            Ok(json!({ "linked": linked }))
        }
        Request::AiStarted { tab_id, site } => {
            // The reply belongs with what was being worked on here when it was asked for.
            let device = state.device.id.clone();
            let task = commands::task_views(&state)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|t| t.in_progress && t.in_progress_on.contains(&device))
                .max_by_key(|t| t.started_at);
            let (task_id, title) = task.map(|t| (t.task.id, t.task.title)).unwrap_or_default();
            eprintln!("AI reply started on {site} in tab {tab_id}");
            crate::jobs::add(app, task_id, title, site, crate::jobs::What::Tab { tab_id });
            Ok(json!({}))
        }
        Request::AiDone { tab_id, looking } => {
            crate::jobs::tab_done(app, tab_id, looking);
            Ok(json!({}))
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

/// What a relay says first, so the app knows it can be sent messages unasked.
const RELAY_HELLO: &str = r#"{"relay":2}"#;

/// Each connected extension's outgoing line queue, so MultiTasker can tell it things unasked.
static EXTENSIONS: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<String>>> = Mutex::new(Vec::new());

/// Asks the browser to bring these pages up: switching to tabs already open and opening the rest beside them.
pub fn open_tabs(urls: &[String]) -> bool {
    push(json!({ "push": "open_tabs", "urls": urls }).to_string())
}

/// Brings a tab to the front in the browser it's in.
pub fn focus_tab(tab_id: i64) -> bool {
    // Windows keeps a program from jumping in front of the one being used; this one was just clicked, so it can let
    // the browser come forward.
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY);
    }
    let sent = push(json!({ "push": "focus_tab", "tab_id": tab_id }).to_string());
    eprintln!("asked the browser to show tab {tab_id}: {}", if sent { "sent" } else { "no browser connected" });
    sent
}

fn push(message: String) -> bool {
    let mut senders = EXTENSIONS.lock().unwrap_or_else(|e| e.into_inner());
    senders.retain(|s| !s.is_closed());
    // One browser is enough; the most recently connected is likely the one in use.
    senders.last().is_some_and(|s| s.send(message).is_ok())
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
                // Replies and messages sent unasked share one queue, so their lines never interleave.
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                tauri::async_runtime::spawn(async move {
                    while let Some(line) = rx.recv().await {
                        if write.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                            break;
                        }
                    }
                });
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Only a relay that says it passes on messages sent unasked gets them; an older one would take one
                    // for the answer to its next question.
                    if line.trim() == RELAY_HELLO {
                        eprintln!("browser relay connected");
                        EXTENSIONS.lock().unwrap_or_else(|e| e.into_inner()).push(tx.clone());
                        continue;
                    }
                    if tx.send(answer(&app, &line).await).is_err() {
                        break;
                    }
                }
                // The relay is gone; a message sent to it now would vanish without a word.
                EXTENSIONS.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| !s.same_channel(&tx));
                eprintln!("browser relay disconnected");
            });
        }
    });
}

#[cfg(not(windows))]
pub fn serve(_app: &AppHandle) {}

/// Requests that are worth starting MultiTasker for; a tab switch alone isn't.
fn starts_app(message: &Value) -> bool {
    !matches!(message.get("type").and_then(|t| t.as_str()), Some("active_tab" | "ai_started" | "ai_done"))
}

/// Writes one message to Chrome, length first, from whichever thread has one.
fn to_browser(stdout: &Mutex<std::io::Stdout>, text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut out = stdout.lock().unwrap_or_else(|e| e.into_inner());
    out.write_all(&(bytes.len() as u32).to_le_bytes()).and_then(|_| out.write_all(bytes)).and_then(|_| out.flush()).is_ok()
}

/// The relay: passes Chrome's length-prefixed messages to the running app, and everything the app sends back, replies
/// and messages sent unasked alike, to Chrome. The pipe is read and written at the same time, which needs asynchronous
/// I/O: a plain file handle to a pipe does one thing at a time, so a pending read would hold up the next message.
pub fn relay() {
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() else { return };
    runtime.block_on(relay_messages());
}

#[cfg(windows)]
type PipeWriter = tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>;

/// The relay's way to the app: the pipe's writing end, and which connection it is, so a reader that ends only clears
/// its own.
#[cfg(windows)]
#[derive(Default)]
struct Link {
    writer: Option<PipeWriter>,
    generation: u64,
}

#[cfg(windows)]
type SharedLink = Arc<tokio::sync::Mutex<Link>>;

#[cfg(windows)]
async fn relay_messages() {
    // Chrome's messages arrive on stdin, read on a plain thread and handed over one at a time.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        loop {
            let mut len = [0u8; 4];
            if stdin.read_exact(&mut len).is_err() {
                return;
            }
            let mut message = vec![0u8; u32::from_le_bytes(len) as usize];
            if stdin.read_exact(&mut message).is_err() || tx.send(message).is_err() {
                return;
            }
        }
    });

    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let link: SharedLink = Arc::default();

    // Stays connected whenever the app runs, so what the app sends unasked (going back to a tab) always gets through,
    // even when the app has restarted and the browser hasn't said anything since.
    {
        let (link, stdout) = (link.clone(), stdout.clone());
        tokio::spawn(async move {
            loop {
                if link.lock().await.writer.is_none() {
                    if let Ok(client) = open_pipe().await {
                        let _ = attach(&link, &stdout, client).await;
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    while let Some(message) = rx.recv().await {
        let parsed: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
        if let Err(error) = send(&link, &stdout, &message, starts_app(&parsed)).await {
            let reply = json!({ "id": parsed.get("id"), "error": error, "offline": true }).to_string();
            if !to_browser(&stdout, &reply) {
                return;
            }
        }
    }
}

#[cfg(not(windows))]
async fn relay_messages() {}

/// Takes a new connection to the app: says hello, passes on whatever comes back, and forgets it when it ends.
#[cfg(windows)]
async fn attach(
    link: &SharedLink,
    stdout: &Arc<Mutex<std::io::Stdout>>,
    client: tokio::net::windows::named_pipe::NamedPipeClient,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let (read, mut write) = tokio::io::split(client);
    write_line(&mut write, RELAY_HELLO.as_bytes()).await.map_err(|e| e.to_string())?;
    let generation = {
        let mut held = link.lock().await;
        held.generation += 1;
        held.writer = Some(write);
        held.generation
    };
    let (link, stdout) = (link.clone(), stdout.clone());
    tokio::spawn(async move {
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = if line.len() > MAX_REPLY {
                let id = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("id").cloned());
                json!({ "id": id, "error": "MultiTasker's answer was too big to pass on." }).to_string()
            } else {
                line
            };
            if !to_browser(&stdout, &line) {
                std::process::exit(0);
            }
        }
        let mut held = link.lock().await;
        if held.generation == generation {
            held.writer = None;
        }
    });
    Ok(())
}

/// Sends one message to the app, connecting first if needed; whatever comes back is passed on as it arrives.
#[cfg(windows)]
async fn send(link: &SharedLink, stdout: &Arc<Mutex<std::io::Stdout>>, message: &[u8], start: bool) -> Result<(), String> {
    {
        let mut held = link.lock().await;
        if let Some(writer) = held.writer.as_mut() {
            if write_line(writer, message).await.is_ok() {
                return Ok(());
            }
            held.writer = None;
        }
    }
    // Not connected yet, or the app restarted since the last message.
    attach(link, stdout, connect(start).await?).await?;
    let mut held = link.lock().await;
    let writer = held.writer.as_mut().ok_or("MultiTasker closed the connection.")?;
    write_line(writer, message).await.map_err(|e| e.to_string())
}

#[cfg(windows)]
async fn write_line(write: &mut (impl tokio::io::AsyncWrite + Unpin), message: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    write.write_all(message).await?;
    write.write_all(b"\n").await?;
    write.flush().await
}

/// Opens the app's pipe, waiting briefly when every instance is busy taking another connection.
#[cfg(windows)]
async fn open_pipe() -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    let mut tries = 0;
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(PIPE) {
            // ERROR_PIPE_BUSY: the app is setting up the next instance.
            Err(e) if e.raw_os_error() == Some(231) && tries < 10 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => return other,
        }
    }
}

#[cfg(windows)]
async fn connect(start: bool) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, String> {
    if let Ok(pipe) = open_pipe().await {
        return Ok(pipe);
    }
    if !start {
        return Err("MultiTasker isn't running.".into());
    }
    std::process::Command::new(app_exe()?).spawn().map_err(|e| format!("Couldn't start MultiTasker: {e}"))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if let Ok(pipe) = open_pipe().await {
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
        "allowed_origins": EXTENSION_IDS.iter().map(|id| format!("chrome-extension://{id}/")).collect::<Vec<_>>(),
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
        assert!(matches!(e.request, Request::ActiveTab { url: None, .. }));
    }

    #[test]
    fn only_deliberate_requests_start_the_app() {
        assert!(!starts_app(&json!({ "type": "active_tab", "url": "https://x" })));
        assert!(starts_app(&json!({ "type": "tasks" })));
        assert!(!starts_app(&json!({ "type": "ai_done", "tab_id": 4, "looking": false })));
    }
}
