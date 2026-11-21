use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Tells the windows a project command started, stopped, printed its address or ended.
pub const PROJECT_RUNS: &str = "project-runs";
/// How much of a command's output is kept to show.
const KEEP_LINES: usize = 400;

/// Something a project folder can run, found from its own files.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectCommand {
    /// Stable across restarts: the folder and the command line.
    pub id: String,
    pub folder: String,
    /// The folder's name, to tell projects apart.
    pub project: String,
    /// What it's for, e.g. "dev", "build", "test".
    pub label: String,
    pub command: String,
}

fn command(folder: &Path, label: &str, line: String) -> ProjectCommand {
    let folder_text = folder.display().to_string();
    ProjectCommand {
        id: format!("{folder_text}\u{1f}{line}"),
        project: folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| folder_text.clone()),
        folder: folder_text,
        label: label.to_owned(),
        command: line,
    }
}

/// Scripts worth showing first, in this order; the rest follow by name.
const FIRST_SCRIPTS: &[&str] = &["dev", "start", "serve", "preview", "build", "test", "lint"];

/// What a folder can run, from its package.json, Cargo.toml and the like.
pub fn detect(folder: &Path) -> Vec<ProjectCommand> {
    let mut out = Vec::new();

    if let Some(scripts) = std::fs::read_to_string(folder.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| json.get("scripts").and_then(|s| s.as_object()).cloned())
    {
        let runner = if folder.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if folder.join("yarn.lock").exists() {
            "yarn"
        } else if folder.join("bun.lockb").exists() || folder.join("bun.lock").exists() {
            "bun run"
        } else {
            "npm run"
        };
        let mut names: Vec<&String> = scripts.keys().collect();
        names.sort_by_key(|n| (FIRST_SCRIPTS.iter().position(|f| f == n).unwrap_or(usize::MAX), n.to_string()));
        for name in names {
            out.push(command(folder, name, format!("{runner} {name}")));
        }
    }

    if folder.join("Cargo.toml").exists() {
        let tauri = folder.join("src-tauri").exists() || folder.join("tauri.conf.json").exists();
        if !tauri {
            out.push(command(folder, "run", "cargo run".into()));
        }
        out.push(command(folder, "build", "cargo build --release".into()));
        out.push(command(folder, "test", "cargo test".into()));
    }

    if folder.join("manage.py").exists() {
        out.push(command(folder, "run", "python manage.py runserver".into()));
    } else if let Some(entry) = ["main.py", "app.py"].iter().find(|f| folder.join(f).exists()) {
        out.push(command(folder, "run", format!("python {entry}")));
    }

    if folder.join("go.mod").exists() {
        out.push(command(folder, "run", "go run .".into()));
        out.push(command(folder, "test", "go test ./...".into()));
    }

    if ["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"].iter().any(|f| folder.join(f).exists()) {
        out.push(command(folder, "up", "docker compose up".into()));
    }
    out
}

/// A command that was started from MultiTasker, running or finished.
struct Run {
    task_id: String,
    command: ProjectCommand,
    child: Option<Child>,
    pid: u32,
    output: VecDeque<String>,
    url: Option<String>,
    started_at: DateTime<Utc>,
    exit: Option<i32>,
    /// Started because its task started, so it stops when the task pauses.
    with_task: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunView {
    pub command_id: String,
    pub task_id: String,
    pub label: String,
    pub project: String,
    pub command: String,
    pub running: bool,
    pub url: Option<String>,
    pub output: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub exit: Option<i32>,
}

#[derive(Default)]
pub struct Projects {
    runs: Mutex<HashMap<String, Run>>,
}

impl Projects {
    fn runs(&self) -> MutexGuard<'_, HashMap<String, Run>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn views(&self) -> Vec<RunView> {
        let mut runs = self.runs();
        let mut views: Vec<RunView> = runs
            .values_mut()
            .map(|r| {
                // A command that ended on its own is noticed here.
                if r.exit.is_none() {
                    if let Some(Ok(Some(status))) = r.child.as_mut().map(|c| c.try_wait()) {
                        r.exit = Some(status.code().unwrap_or(-1));
                    }
                }
                RunView {
                    command_id: r.command.id.clone(),
                    task_id: r.task_id.clone(),
                    label: r.command.label.clone(),
                    project: r.command.project.clone(),
                    command: r.command.command.clone(),
                    running: r.exit.is_none(),
                    url: r.url.clone(),
                    output: r.output.iter().cloned().collect(),
                    started_at: r.started_at,
                    exit: r.exit,
                }
            })
            .collect();
        views.sort_by_key(|v| v.started_at);
        views
    }

    /// Starts a command in its folder without a console window, keeping its output and the local address it prints.
    pub fn start(self: &Arc<Self>, app: &AppHandle, task_id: &str, command: ProjectCommand, with_task: bool) -> Result<(), String> {
        if self.runs().get(&command.id).is_some_and(|r| r.exit.is_none()) {
            return Ok(());
        }
        let mut child = shell(&command.command, Path::new(&command.folder))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Couldn't run {}: {e}", command.command))?;
        let pid = child.id();
        let id = command.id.clone();
        for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
            let (projects, app, id) = (self.clone(), app.clone(), id.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    projects.note_line(&app, &id, pid, &line);
                }
                projects.note_ended(&app, &id, pid);
            });
        }
        self.runs().insert(
            id,
            Run {
                task_id: task_id.to_owned(),
                command,
                child: Some(child),
                pid,
                output: VecDeque::new(),
                url: None,
                started_at: Utc::now(),
                exit: None,
                with_task,
            },
        );
        let _ = app.emit(PROJECT_RUNS, ());
        Ok(())
    }

    fn note_line(&self, app: &AppHandle, id: &str, pid: u32, line: &str) {
        let line = strip_ansi(line);
        let mut runs = self.runs();
        let Some(run) = runs.get_mut(id).filter(|r| r.pid == pid) else { return };
        let found = run.url.is_none().then(|| local_url(&line)).flatten();
        run.output.push_back(line);
        while run.output.len() > KEEP_LINES {
            run.output.pop_front();
        }
        if let Some(url) = found {
            run.url = Some(url);
            drop(runs);
            let _ = app.emit(PROJECT_RUNS, ());
        }
    }

    fn note_ended(&self, app: &AppHandle, id: &str, pid: u32) {
        let mut runs = self.runs();
        let Some(run) = runs.get_mut(id).filter(|r| r.pid == pid) else { return };
        if let Some(Ok(status)) = run.child.as_mut().map(|c| c.wait()) {
            run.exit.get_or_insert(status.code().unwrap_or(-1));
        }
        drop(runs);
        let _ = app.emit(PROJECT_RUNS, ());
    }

    /// Stops a command and everything it started (a dev server's own children too).
    pub fn stop(&self, app: &AppHandle, id: &str) {
        let pid = self.runs().get(id).filter(|r| r.exit.is_none()).map(|r| r.pid);
        if let Some(pid) = pid {
            kill_tree(pid);
            if let Some(run) = self.runs().get_mut(id) {
                run.exit.get_or_insert(-1);
            }
        }
        let _ = app.emit(PROJECT_RUNS, ());
    }

    /// Stops what a task started along with itself, now that it has paused or finished.
    pub fn stop_with_task(&self, app: &AppHandle, task_id: &str) {
        let ids: Vec<String> = self.runs().values().filter(|r| r.task_id == task_id && r.with_task && r.exit.is_none()).map(|r| r.command.id.clone()).collect();
        for id in ids {
            self.stop(app, &id);
        }
    }

    /// Stops everything, for when MultiTasker quits.
    pub fn stop_all(&self) {
        let pids: Vec<u32> = self.runs().values().filter(|r| r.exit.is_none()).map(|r| r.pid).collect();
        for pid in pids {
            kill_tree(pid);
        }
    }

    pub fn url(&self, id: &str) -> Option<String> {
        self.runs().get(id).and_then(|r| r.url.clone())
    }
}

#[cfg(windows)]
fn shell(line: &str, folder: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = Command::new("cmd");
    c.arg("/C").raw_arg(line).current_dir(folder).creation_flags(CREATE_NO_WINDOW);
    c
}

#[cfg(not(windows))]
fn shell(line: &str, folder: &Path) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(line).current_dir(folder);
    c
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).creation_flags(0x0800_0000).output();
}

#[cfg(not(windows))]
fn kill_tree(pid: u32) {
    let _ = Command::new("kill").args(["-TERM", &format!("-{pid}")]).output();
}

/// Terminal colours and cursor codes, which a dev server prints but a plain view can't show.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The first local address a dev server prints, as a browser can open it.
fn local_url(line: &str) -> Option<String> {
    for start in line.match_indices("http").map(|(i, _)| i) {
        let rest = &line[start..];
        let end = rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ')' || c == ',').unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(['.', '/']);
        let host = url.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
        if matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]") && url.split("://").nth(1).is_some_and(|r| r.contains(':')) {
            return Some(url.replace("0.0.0.0", "localhost"));
        }
    }
    None
}

/// Folders linked to a task that are on this computer.
pub fn task_folders(attachments: &[multitasker_core::Attachment], device: &str) -> Vec<PathBuf> {
    attachments
        .iter()
        .filter(|a| a.kind == multitasker_core::AttachmentKind::Folder)
        .filter(|a| a.device.as_deref().is_none_or(|d| d == device))
        .map(|a| PathBuf::from(&a.target))
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_a_folder_can_run() {
        let dir = std::env::temp_dir().join(format!("mt-project-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), r#"{"scripts":{"lint":"x","build":"x","dev":"x","zz":"x"}}"#).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        let found: Vec<String> = detect(&dir).into_iter().map(|c| c.command).collect();
        assert_eq!(found, ["pnpm dev", "pnpm build", "pnpm lint", "pnpm zz"], "the usual scripts first, the rest by name");

        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        assert!(detect(&dir).iter().any(|c| c.command == "cargo run"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_address_a_dev_server_prints() {
        assert_eq!(local_url("  ➜  Local:   http://localhost:5173/").as_deref(), Some("http://localhost:5173"));
        assert_eq!(local_url("Listening on http://0.0.0.0:3000.").as_deref(), Some("http://localhost:3000"));
        assert_eq!(local_url("docs at https://vitejs.dev/config"), None);
        assert_eq!(strip_ansi("\u{1b}[32mready\u{1b}[39m in 300 ms"), "ready in 300 ms");
    }

    #[cfg(windows)]
    #[test]
    fn stopping_ends_the_command_and_what_it_started() {
        let mut child = shell("ping -n 30 127.0.0.1 > nul", &std::env::temp_dir()).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(child.try_wait().unwrap().is_none(), "still running");
        let started = std::time::Instant::now();
        kill_tree(child.id());
        child.wait().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "ended well before ping would have");
    }

    #[cfg(windows)]
    #[test]
    fn runs_a_command_and_keeps_its_output() {
        let mut child = shell("echo hello from a project", &std::env::temp_dir()).stdout(Stdio::piped()).spawn().unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut text).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(text.trim(), "hello from a project");
    }
}
