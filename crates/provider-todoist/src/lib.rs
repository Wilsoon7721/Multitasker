use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use multitasker_core::{
    check_category, check_title, LeftOff, clean, content_type, multipart_file, Attachment, AttachmentKind, DeviceId, Error,
    ExtrasStore, HandedInFile, NewTask, Result, Task, TaskChanges, TaskExtras, TaskId, TaskProvider, TaskStatus, WorkStore,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

pub const PROVIDER_KEY: &str = "todoist";

const API: &str = "https://api.todoist.com/api/v1";

/// A list this fresh is reused, so the UI, detection and reminders don't each ask Todoist every few seconds.
const CACHE_FOR: Duration = Duration::from_secs(15);

/// Finished tasks this recent still show, like recently done ones in the other task lists.
const DONE_FOR_DAYS: i64 = 14;

/// The label that marks a task in progress, so it shows (and can be filtered) in Todoist too.
pub const IN_PROGRESS_LABEL: &str = "in-progress";

/// Ends every comment MultiTasker writes, so it can find its own among the person's.
const MARK: &str = "MultiTasker";

/// The largest file MultiTasker downloads from Todoist in one go.
const MAX_FILE: u64 = 200 * 1024 * 1024;

/// Tasks kept in Todoist, read and written with a personal API token.
pub struct Todoist {
    inner: Arc<Inner>,
}

struct Inner {
    token: String,
    agent: ureq::Agent,
    cache: Mutex<Option<(Instant, Snapshot)>>,
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    active: Vec<Item>,
    done: Vec<Item>,
    notes: Vec<Note>,
    projects: Vec<Project>,
}

#[derive(Debug, Clone, Deserialize)]
struct Item {
    id: String,
    content: String,
    #[serde(default)]
    description: String,
    project_id: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    checked: bool,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    due: Option<Due>,
    added_at: DateTime<Utc>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Due {
    date: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Note {
    id: String,
    item_id: String,
    content: String,
    posted_at: DateTime<Utc>,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    file_attachment: Option<FileAttachment>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileAttachment {
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    file_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Project {
    id: String,
    name: String,
    #[serde(default)]
    inbox_project: bool,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    is_archived: bool,
}

impl Todoist {
    pub fn new(token: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self { inner: Arc::new(Inner { token: token.into().trim().to_owned(), agent, cache: Mutex::new(None) }) }
    }

    /// Checks the token with Todoist, returning the account's name for Settings.
    pub async fn account(&self) -> Result<String> {
        self.call(|inner| {
            let body = inner.sync(&["user"])?;
            let user = body.get("user").ok_or_else(|| Error::Storage("Todoist sent no account back.".into()))?;
            let name = ["full_name", "email"]
                .iter()
                .filter_map(|k| user.get(*k).and_then(|v| v.as_str()))
                .find(|v| !v.trim().is_empty())
                .unwrap_or("your Todoist account");
            Ok(name.to_owned())
        })
        .await
    }

    /// Runs a blocking API call off the async runtime's worker threads.
    async fn call<T: Send + 'static>(&self, f: impl FnOnce(&Inner) -> Result<T> + Send + 'static) -> Result<T> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || f(&inner)).await.map_err(|e| Error::Storage(e.to_string()))?
    }
}

impl Inner {
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{API}{path}");
        let auth = format!("Bearer {}", self.token);
        let response = match (method, body) {
            ("GET", _) => self.agent.get(&url).header("authorization", &auth).call(),
            ("DELETE", _) => self.agent.delete(&url).header("authorization", &auth).call(),
            (_, Some(body)) => self
                .agent
                .post(&url)
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .send(body.to_string().as_bytes()),
            (_, None) => self.agent.post(&url).header("authorization", &auth).send_empty(),
        };
        Self::read(response)
    }

    /// One request for everything the sync API keeps: open tasks, their comments, projects.
    fn sync(&self, resources: &[&str]) -> Result<Value> {
        let resources = serde_json::to_string(resources).map_err(|e| Error::Storage(e.to_string()))?;
        let response = self
            .agent
            .post(&format!("{API}/sync"))
            .header("authorization", &format!("Bearer {}", self.token))
            .send_form([("sync_token", "*"), ("resource_types", resources.as_str())]);
        Self::read(response)
    }

    /// Uploads a file to Todoist's file store, returning what a comment needs to carry it.
    fn upload(&self, name: &str, bytes: &[u8]) -> Result<Value> {
        let (body, content_type) = multipart_file("file", name, content_type(name), bytes);
        let response = self
            .agent
            .post(&format!("{API}/uploads"))
            .header("authorization", &format!("Bearer {}", self.token))
            .header("content-type", &content_type)
            .send(&body[..]);
        Self::read(response)
    }

    /// A file Todoist stores, following its redirect to wherever the content is served from.
    fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(url)
            .header("authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(|e| Error::Storage(format!("Couldn't download it from Todoist: {e}")))?;
        if !response.status().is_success() {
            return Err(Error::Storage(format!("Todoist answered {} for that file.", response.status().as_u16())));
        }
        response
            .body_mut()
            .with_config()
            .limit(MAX_FILE)
            .read_to_vec()
            .map_err(|e| Error::Storage(format!("Couldn't download it from Todoist: {e}")))
    }

    fn read(response: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
        let mut response = response.map_err(|e| Error::Storage(format!("Couldn't reach Todoist: {e}")))?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().map_err(|e| Error::Storage(e.to_string()))?;
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let message = parsed.get("error").and_then(|e| e.as_str()).map(str::to_owned);
        match status {
            200..=299 => Ok(parsed),
            401 | 403 => Err(Error::Storage("Todoist turned the token down; paste a new one in Settings.".into())),
            404 => Err(Error::NotFound(message.unwrap_or_else(|| "That task no longer exists in Todoist.".into()))),
            429 => Err(Error::Storage("Todoist asked MultiTasker to slow down; try again in a minute.".into())),
            400..=499 => Err(Error::Invalid(message.unwrap_or_else(|| format!("Todoist refused that ({status}).")))),
            _ => Err(Error::Storage(message.unwrap_or_else(|| format!("Todoist answered {status}.")))),
        }
    }

    fn forget_cache(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn snapshot(&self) -> Result<Snapshot> {
        if let Some((at, snapshot)) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if at.elapsed() < CACHE_FOR {
                return Ok(snapshot.clone());
            }
        }
        let body = self.sync(&["items", "notes", "projects"])?;
        let list = |key: &str| body.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let parse = |key: &str| -> Vec<Value> { list(key) };

        let now = Utc::now();
        let since = now - chrono::Duration::days(DONE_FOR_DAYS);
        let fmt = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
        let done = self.send(
            "GET",
            &format!(
                "/tasks/completed/by_completion_date?since={}&until={}&limit=200",
                fmt(since),
                fmt(now + chrono::Duration::minutes(5))
            ),
            None,
        )?;

        let snapshot = Snapshot {
            active: parse("items")
                .into_iter()
                .filter_map(|v| serde_json::from_value::<Item>(v).ok())
                .filter(|i| !i.is_deleted && !i.checked)
                .collect(),
            done: done
                .get("items")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| serde_json::from_value::<Item>(v.clone()).ok())
                .filter(|i| !i.is_deleted)
                .collect(),
            notes: parse("notes")
                .into_iter()
                .filter_map(|v| serde_json::from_value::<Note>(v).ok())
                .filter(|n| !n.is_deleted)
                .collect(),
            projects: parse("projects")
                .into_iter()
                .filter_map(|v| serde_json::from_value::<Project>(v).ok())
                .filter(|p| !p.is_deleted)
                .collect(),
        };
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), snapshot.clone()));
        Ok(snapshot)
    }

    /// The project a category belongs in: the Inbox for none, an existing one by name, or a new one.
    fn project_for(&self, category: Option<&str>) -> Result<String> {
        let snapshot = self.snapshot()?;
        let live = snapshot.projects.iter().filter(|p| !p.is_archived);
        let found = match category {
            None => live.clone().find(|p| p.inbox_project),
            Some(name) => live.clone().find(|p| !p.inbox_project && p.name.eq_ignore_ascii_case(name)),
        };
        if let Some(project) = found {
            return Ok(project.id.clone());
        }
        let Some(name) = category else {
            return Err(Error::Storage("Todoist has no Inbox project to put the task in.".into()));
        };
        let created = self.send("POST", "/projects", Some(&json!({ "name": name })))?;
        self.forget_cache();
        created
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| Error::Storage("Todoist sent no project back.".into()))
    }

    /// The task as it now stands, read fresh after a change.
    fn fresh(&self, id: &str) -> Result<Task> {
        self.forget_cache();
        let snapshot = self.snapshot()?;
        snapshot
            .active
            .iter()
            .chain(&snapshot.done)
            .find(|i| i.id == id)
            .map(|i| task_of(i, &snapshot.projects))
            .ok_or_else(|| Error::NotFound("That task no longer exists in Todoist.".into()))
    }

    fn set_labels(&self, id: &str, labels: Vec<String>) -> Result<()> {
        self.send("POST", &format!("/tasks/{id}"), Some(&json!({ "labels": labels }))).map(|_| ())
    }

    /// Takes the in-progress label and MultiTasker's "started" comments off a task.
    fn stop(&self, id: &str) -> Result<()> {
        let snapshot = self.snapshot()?;
        let Some(item) = snapshot.active.iter().find(|i| i.id == id) else { return Ok(()) };
        if item.labels.iter().any(|l| l == IN_PROGRESS_LABEL) {
            self.set_labels(id, item.labels.iter().filter(|l| *l != IN_PROGRESS_LABEL).cloned().collect())?;
        }
        for note in snapshot.notes.iter().filter(|n| n.item_id == id) {
            if matches!(parse_note(note), Some(Mark::Started { .. })) {
                self.send("DELETE", &format!("/comments/{}", note.id), None)?;
            }
        }
        self.forget_cache();
        Ok(())
    }
}

fn task_of(item: &Item, projects: &[Project]) -> Task {
    let (due_date, due_at) = due_of(item.due.as_ref());
    let category = projects.iter().find(|p| p.id == item.project_id && !p.inbox_project).map(|p| p.name.clone());
    Task {
        id: item.id.clone(),
        title: item.content.clone(),
        description: clean(Some(item.description.clone())),
        category,
        due_date,
        due_at,
        status: if item.checked || item.completed_at.is_some() { TaskStatus::Done } else { TaskStatus::Todo },
        created_at: item.added_at,
        updated_at: item.updated_at.unwrap_or(item.added_at),
        completed_at: item.completed_at,
    }
}

/// Todoist's due dates come as a plain date, a UTC moment, or a "floating" local time with no zone.
fn due_of(due: Option<&Due>) -> (Option<NaiveDate>, Option<DateTime<Utc>>) {
    let Some(raw) = due.map(|d| d.date.as_str()) else { return (None, None) };
    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return (Some(date), None);
    }
    let at = DateTime::parse_from_rfc3339(raw).map(|t| t.with_timezone(&Utc)).ok().or_else(|| {
        let local = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S").ok()?;
        Local.from_local_datetime(&local).earliest().map(|t| t.with_timezone(&Utc))
    });
    match at {
        Some(at) => (Some(at.with_timezone(&Local).date_naive()), Some(at)),
        None => (None, None),
    }
}

/// How a due date or time goes into Todoist, given what the task has now.
fn due_body(changes: &TaskChanges, current: Option<NaiveDate>) -> Option<(&'static str, Value)> {
    match (changes.due_at, changes.due_date) {
        (Some(Some(at)), _) => Some(("due_datetime", json!(at.to_rfc3339_opts(SecondsFormat::Secs, true)))),
        (_, Some(Some(date))) => Some(("due_date", json!(date))),
        (_, Some(None)) => Some(("due_string", json!("no date"))),
        // Taking the time off keeps the day.
        (Some(None), None) => Some(match current {
            Some(date) => ("due_date", json!(date)),
            None => ("due_string", json!("no date")),
        }),
        (None, None) => None,
    }
}

/// A comment MultiTasker wrote, read back.
#[derive(Debug, PartialEq)]
enum Mark {
    Started { device: Option<String>, device_name: Option<String> },
    Attachment(Attachment),
    HandedIn(HandedInFile),
    /// The task's time spent, in seconds, kept in one comment that's updated.
    Time(i64),
    LeftOff(LeftOff),
}

fn time_note(secs: i64) -> String {
    let minutes = secs / 60;
    let shown = if minutes < 60 { format!("{minutes}m") } else { format!("{}h {}m", minutes / 60, minutes % 60) };
    format!("⏱ Time spent: {shown}\n\n{MARK} · time · {secs}")
}

fn left_off_note(note: &LeftOff) -> String {
    format!("📝 Left off: {}\n\n{MARK} · left-off · {}", one_line(&note.text), note.device)
}

fn handed_in_note(file: &HandedInFile, device_name: &str) -> String {
    format!(
        "📦 Handed in {} from {}\n\n{MARK} · handed-in · {} · {} · {}",
        one_line(&file.name),
        one_line(device_name),
        file.sha256,
        file.size,
        file.device.as_deref().unwrap_or("-")
    )
}

fn started_note(device: &DeviceId, device_name: &str) -> String {
    format!("▶ Started on {}\n\n{MARK} · started · {device}", one_line(device_name))
}

fn attachment_note(a: &Attachment) -> String {
    if a.kind == AttachmentKind::Link {
        return format!("📎 [{}]({})\n\n{MARK} · link", one_line(&a.name).replace(['[', ']'], ""), a.target);
    }
    let device = a.device.as_deref().unwrap_or("-");
    format!("📎 {}\n`{}`\n\n{MARK} · {} · {device}", one_line(&a.name), a.target, a.kind.as_str())
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_note(note: &Note) -> Option<Mark> {
    let lines: Vec<&str> = note.content.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let footer = lines.last()?.strip_prefix(MARK)?.strip_prefix(" · ")?;
    let parts: Vec<&str> = footer.split(" · ").collect();
    let first = lines.first()?;

    if parts[0] == "handed-in" {
        let name = note
            .file_attachment
            .as_ref()
            .and_then(|f| f.file_name.clone())
            .or_else(|| first.strip_prefix("📦 Handed in ").and_then(|r| r.rsplit_once(" from ")).map(|(n, _)| n.to_owned()))?;
        return Some(Mark::HandedIn(HandedInFile {
            id: note.id.clone(),
            task_id: note.item_id.clone(),
            name,
            sha256: parts.get(1)?.to_string(),
            size: parts.get(2)?.parse().ok()?,
            device: parts.get(3).filter(|d| **d != "-").map(|d| d.to_string()),
            added_at: note.posted_at,
        }));
    }

    if parts[0] == "time" {
        return Some(Mark::Time(parts.get(1)?.parse().ok()?));
    }

    if parts[0] == "left-off" {
        return Some(Mark::LeftOff(LeftOff {
            text: first.strip_prefix("📝 Left off: ")?.to_owned(),
            at: note.posted_at,
            device: parts.get(1).map(|d| d.to_string()).unwrap_or_default(),
        }));
    }

    if parts[0] == "started" {
        return Some(Mark::Started {
            device: parts.get(1).map(|d| d.to_string()),
            device_name: first.strip_prefix("▶ Started on ").map(str::to_owned),
        });
    }

    let kind = AttachmentKind::parse(parts[0]).ok()?;
    let first = first.strip_prefix("📎 ")?;
    let (name, target, device) = if kind == AttachmentKind::Link {
        let split = first.rfind("](")?;
        let name = first.get(..split)?.strip_prefix('[')?;
        let url = first.get(split + 2..)?.strip_suffix(')')?;
        (name.to_owned(), url.to_owned(), None)
    } else {
        let target = lines.get(1)?.strip_prefix('`')?.strip_suffix('`')?;
        let device = parts.get(1).filter(|d| **d != "-").map(|d| d.to_string());
        (first.to_owned(), target.to_owned(), device)
    };
    Some(Mark::Attachment(Attachment {
        id: note.id.clone(),
        provider: PROVIDER_KEY.into(),
        task_id: note.item_id.clone(),
        kind,
        name,
        target,
        device,
        added_at: note.posted_at,
    }))
}

fn extras_of(item: &Item, notes: &[Note]) -> TaskExtras {
    let mut started: Option<(&Note, Option<String>, Option<String>)> = None;
    let mut attachments = Vec::new();
    let mut worked_secs = 0;
    let mut left_off: Option<LeftOff> = None;
    for note in notes.iter().filter(|n| n.item_id == item.id) {
        match parse_note(note) {
            Some(Mark::Started { device, device_name }) => {
                if started.as_ref().is_none_or(|(first, ..)| note.posted_at < first.posted_at) {
                    started = Some((note, device, device_name));
                }
            }
            Some(Mark::Attachment(a)) => attachments.push(a),
            Some(Mark::Time(secs)) => worked_secs = worked_secs.max(secs),
            Some(Mark::LeftOff(n)) => {
                if left_off.as_ref().is_none_or(|l| n.at > l.at) {
                    left_off = Some(n);
                }
            }
            Some(Mark::HandedIn(_)) | None => {}
        }
    }
    attachments.sort_by_key(|a| a.added_at);

    // The label decides, so adding or removing it in Todoist itself starts or stops the task.
    if !item.labels.iter().any(|l| l == IN_PROGRESS_LABEL) {
        return TaskExtras { attachments, worked_secs, left_off, ..Default::default() };
    }
    let (started_at, started_on, started_on_name) = match started {
        Some((note, device, name)) => (note.posted_at, device, name),
        None => (item.updated_at.unwrap_or(item.added_at), None, None),
    };
    TaskExtras { started_at: Some(started_at), started_on, started_on_name, attachments, worked_secs, left_off }
}

#[async_trait]
impl TaskProvider for Todoist {
    fn key(&self) -> &str {
        PROVIDER_KEY
    }

    fn display_name(&self) -> &str {
        "Todoist"
    }

    fn extras(&self) -> Option<&dyn ExtrasStore> {
        Some(self)
    }

    fn work(&self) -> Option<&dyn WorkStore> {
        Some(self)
    }

    async fn list(&self) -> Result<Vec<Task>> {
        self.call(|inner| {
            let s = inner.snapshot()?;
            Ok(s.active.iter().chain(&s.done).map(|i| task_of(i, &s.projects)).collect())
        })
        .await
    }

    async fn create(&self, task: NewTask) -> Result<Task> {
        let title = check_title(&task.title)?;
        let category = check_category(task.category)?;
        let description = clean(task.description);
        self.call(move |inner| {
            let mut body = Map::new();
            body.insert("content".into(), json!(title));
            if let Some(d) = description {
                body.insert("description".into(), json!(d));
            }
            body.insert("project_id".into(), json!(inner.project_for(category.as_deref())?));
            let due = TaskChanges { due_at: Some(task.due_at), due_date: Some(task.due_date), ..Default::default() };
            if let Some((key, value)) = due_body(&due, None).filter(|(k, _)| *k != "due_string") {
                body.insert(key.into(), value);
            }
            let created = inner.send("POST", "/tasks", Some(&Value::Object(body)))?;
            let id = created.get("id").and_then(|v| v.as_str()).ok_or_else(|| Error::Storage("Todoist sent no task back.".into()))?;
            inner.fresh(id)
        })
        .await
    }

    async fn update(&self, id: &TaskId, changes: TaskChanges) -> Result<Task> {
        let title = changes.title.as_deref().map(check_title).transpose()?;
        let category = changes.category.clone().map(check_category).transpose()?;
        let id = id.clone();
        self.call(move |inner| {
            let snapshot = inner.snapshot()?;
            let current = snapshot.active.iter().chain(&snapshot.done).find(|i| i.id == id);
            let is_done = current.is_some_and(|i| i.checked || i.completed_at.is_some());

            let mut body = Map::new();
            if let Some(title) = title {
                body.insert("content".into(), json!(title));
            }
            if let Some(description) = changes.description.clone() {
                body.insert("description".into(), json!(clean(description).unwrap_or_default()));
            }
            let current_date = current.and_then(|i| due_of(i.due.as_ref()).0);
            if let Some((key, value)) = due_body(&changes, current_date) {
                body.insert(key.into(), value);
            }
            if !body.is_empty() {
                inner.send("POST", &format!("/tasks/{id}"), Some(&Value::Object(body)))?;
            }
            if let Some(category) = category {
                let project = inner.project_for(category.as_deref())?;
                if current.is_none_or(|i| i.project_id != project) {
                    inner.send("POST", &format!("/tasks/{id}/move"), Some(&json!({ "project_id": project })))?;
                }
            }
            match changes.status {
                // A finished task is no longer in progress anywhere.
                Some(TaskStatus::Done) if !is_done => {
                    inner.stop(&id)?;
                    inner.send("POST", &format!("/tasks/{id}/close"), None)?;
                }
                Some(TaskStatus::Todo) if is_done => {
                    inner.send("POST", &format!("/tasks/{id}/reopen"), None)?;
                }
                _ => {}
            }
            inner.fresh(&id)
        })
        .await
    }

    async fn delete(&self, id: &TaskId) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            inner.forget_cache();
            inner.send("DELETE", &format!("/tasks/{id}"), None).map(|_| ())
        })
        .await
    }
}

#[async_trait]
impl ExtrasStore for Todoist {
    async fn extras(&self) -> Result<HashMap<TaskId, TaskExtras>> {
        self.call(|inner| {
            let s = inner.snapshot()?;
            Ok(s
                .active
                .iter()
                .map(|i| (i.id.clone(), extras_of(i, &s.notes)))
                .filter(|(_, e)| e.started_at.is_some() || !e.attachments.is_empty() || e.worked_secs > 0 || e.left_off.is_some())
                .collect())
        })
        .await
    }

    async fn start(&self, id: &TaskId, device: &DeviceId, device_name: &str) -> Result<()> {
        let (id, note) = (id.clone(), started_note(device, device_name));
        self.call(move |inner| {
            let snapshot = inner.snapshot()?;
            let item = snapshot.active.iter().find(|i| i.id == id).ok_or_else(|| Error::NotFound("That task no longer exists in Todoist.".into()))?;
            let started: Vec<&Note> = snapshot
                .notes
                .iter()
                .filter(|n| n.item_id == id && matches!(parse_note(n), Some(Mark::Started { .. })))
                .collect();
            // Starting again keeps when and where it was first started.
            if item.labels.iter().any(|l| l == IN_PROGRESS_LABEL) && !started.is_empty() {
                return Ok(());
            }
            for stale in started {
                inner.send("DELETE", &format!("/comments/{}", stale.id), None)?;
            }
            if !item.labels.iter().any(|l| l == IN_PROGRESS_LABEL) {
                let mut labels = item.labels.clone();
                labels.push(IN_PROGRESS_LABEL.into());
                inner.set_labels(&id, labels)?;
            }
            inner.send("POST", "/comments", Some(&json!({ "task_id": id, "content": note })))?;
            inner.forget_cache();
            Ok(())
        })
        .await
    }

    async fn stop(&self, id: &TaskId) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| inner.stop(&id)).await
    }

    async fn add_attachment(&self, a: Attachment) -> Result<Attachment> {
        let body = json!({ "task_id": a.task_id, "content": attachment_note(&a) });
        self.call(move |inner| {
            let response = inner.send("POST", "/comments", Some(&body))?;
            inner.forget_cache();
            let note: Note = serde_json::from_value(response).map_err(|e| Error::Storage(e.to_string()))?;
            match parse_note(&note) {
                Some(Mark::Attachment(a)) => Ok(a),
                _ => Err(Error::Storage("Todoist changed the attachment so MultiTasker can't read it back.".into())),
            }
        })
        .await
    }

    async fn remove_attachment(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |inner| {
            inner.forget_cache();
            inner.send("DELETE", &format!("/comments/{id}"), None).map(|_| ())
        })
        .await
    }

    async fn add_worked(&self, id: &TaskId, secs: i64) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            // Read fresh, so time another computer just added isn't overwritten.
            inner.forget_cache();
            let snapshot = inner.snapshot()?;
            let kept = snapshot.notes.iter().filter(|n| n.item_id == id).find_map(|n| match parse_note(n) {
                Some(Mark::Time(total)) => Some((n.id.clone(), total)),
                _ => None,
            });
            match kept {
                Some((comment, total)) => {
                    inner.send("POST", &format!("/comments/{comment}"), Some(&json!({ "content": time_note(total + secs) })))?
                }
                None => inner.send("POST", "/comments", Some(&json!({ "task_id": id, "content": time_note(secs) })))?,
            };
            inner.forget_cache();
            Ok(())
        })
        .await
    }

    async fn set_left_off(&self, id: &TaskId, note: Option<LeftOff>) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            let snapshot = inner.snapshot()?;
            let old: Vec<String> = snapshot
                .notes
                .iter()
                .filter(|n| n.item_id == id && matches!(parse_note(n), Some(Mark::LeftOff(_))))
                .map(|n| n.id.clone())
                .collect();
            // A new comment rather than an edit, so its time says when the note was written.
            if let Some(note) = &note {
                inner.send("POST", "/comments", Some(&json!({ "task_id": id, "content": left_off_note(note) })))?;
            }
            for comment in old {
                inner.send("DELETE", &format!("/comments/{comment}"), None)?;
            }
            inner.forget_cache();
            Ok(())
        })
        .await
    }
}

#[derive(Debug, Deserialize)]
struct Comments {
    #[serde(default)]
    results: Vec<Note>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[async_trait]
impl WorkStore for Todoist {
    async fn handed_in(&self, task_id: &TaskId) -> Result<Vec<HandedInFile>> {
        let task_id = task_id.clone();
        self.call(move |inner| {
            let mut out = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..10 {
                let mut path = format!("/comments?task_id={task_id}&limit=200");
                if let Some(c) = &cursor {
                    path.push_str(&format!("&cursor={c}"));
                }
                let page: Comments = serde_json::from_value(inner.send("GET", &path, None)?).map_err(|e| Error::Storage(e.to_string()))?;
                out.extend(page.results.iter().filter(|n| !n.is_deleted).filter_map(|n| match parse_note(n) {
                    Some(Mark::HandedIn(f)) => Some(f),
                    _ => None,
                }));
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            Ok(out)
        })
        .await
    }

    async fn all_handed_in(&self) -> Result<Vec<HandedInFile>> {
        self.call(|inner| {
            Ok(inner
                .snapshot()?
                .notes
                .iter()
                .filter_map(|n| match parse_note(n) {
                    Some(Mark::HandedIn(f)) => Some(f),
                    _ => None,
                })
                .collect())
        })
        .await
    }

    async fn hand_in(&self, file: HandedInFile, bytes: Vec<u8>, device_name: &str) -> Result<HandedInFile> {
        let content = handed_in_note(&file, device_name);
        self.call(move |inner| {
            let uploaded = inner.upload(&file.name, &bytes)?;
            let pick = |k: &str| uploaded.get(k).cloned().unwrap_or(Value::Null);
            let attachment = json!({
                "file_name": file.name,
                "file_type": pick("file_type"),
                "file_url": pick("file_url"),
                "resource_type": pick("resource_type"),
            });
            let body = json!({ "task_id": file.task_id, "content": content, "attachment": attachment });
            let note: Note = serde_json::from_value(inner.send("POST", "/comments", Some(&body))?).map_err(|e| Error::Storage(e.to_string()))?;
            inner.forget_cache();
            match parse_note(&note) {
                Some(Mark::HandedIn(stored)) => Ok(stored),
                _ => Err(Error::Storage("Todoist changed the handed-in file so MultiTasker can't read it back.".into())),
            }
        })
        .await
    }

    async fn remove_handed_in(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |inner| {
            inner.forget_cache();
            match inner.send("DELETE", &format!("/comments/{id}"), None) {
                Ok(_) | Err(Error::NotFound(_)) => Ok(()),
                Err(e) => Err(e),
            }
        })
        .await
    }

    async fn download(&self, file: &HandedInFile) -> Result<Vec<u8>> {
        let id = file.id.clone();
        self.call(move |inner| {
            let note: Note = serde_json::from_value(inner.send("GET", &format!("/comments/{id}"), None)?).map_err(|e| Error::Storage(e.to_string()))?;
            let url = note
                .file_attachment
                .and_then(|f| f.file_url)
                .ok_or_else(|| Error::NotFound("That handed-in file is no longer in Todoist.".into()))?;
            inner.fetch(&url)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(content: &str) -> Note {
        Note {
            id: "n1".into(),
            item_id: "t1".into(),
            content: content.into(),
            posted_at: Utc::now(),
            is_deleted: false,
            file_attachment: None,
        }
    }

    fn attachment(kind: AttachmentKind, name: &str, target: &str, device: Option<&str>) -> Attachment {
        Attachment {
            id: "n1".into(),
            provider: PROVIDER_KEY.into(),
            task_id: "t1".into(),
            kind,
            name: name.into(),
            target: target.into(),
            device: device.map(str::to_owned),
            added_at: Utc::now(),
        }
    }

    #[test]
    fn attachments_round_trip_through_comments() {
        for a in [
            attachment(AttachmentKind::Link, "Claude chat", "https://claude.ai/chat/1?x=(1)", None),
            attachment(AttachmentKind::Document, "GS notes (v2).docx", r"C:\Users\me\School\GS notes (v2).docx", Some("pc-1")),
            attachment(AttachmentKind::App, "Visual Studio Code", r"C:\Program Files\Code\Code.exe", Some("pc-1")),
        ] {
            let mut back = match parse_note(&note(&attachment_note(&a))) {
                Some(Mark::Attachment(back)) => back,
                other => panic!("{a:?} came back as {other:?}"),
            };
            back.added_at = a.added_at;
            assert_eq!(back, a);
        }
    }

    #[test]
    fn time_and_left_off_notes_round_trip_through_comments() {
        let time = time_note(3 * 3600 + 20 * 60 + 5);
        assert!(time.starts_with("⏱ Time spent: 3h 20m"));
        assert_eq!(parse_note(&note(&time)), Some(Mark::Time(12005)));

        let n = note(&left_off_note(&LeftOff { text: "Page 134,\nthe diagram".into(), at: Utc::now(), device: "pc-1".into() }));
        match parse_note(&n) {
            Some(Mark::LeftOff(back)) => assert_eq!((back.text.as_str(), back.device.as_str()), ("Page 134, the diagram", "pc-1")),
            other => panic!("came back as {other:?}"),
        }

        let item: Item = serde_json::from_value(serde_json::json!({
            "id": "t1", "project_id": "p1", "content": "Essay", "labels": [], "added_at": "2026-11-20T00:00:00Z", "checked": false
        }))
        .unwrap();
        let extras = extras_of(&item, &[note(&time), n]);
        assert_eq!(extras.worked_secs, 12005);
        assert_eq!(extras.left_off.map(|l| l.text), Some("Page 134, the diagram".into()));
    }

    #[test]
    fn reads_started_marks_and_ignores_the_persons_own_comments() {
        assert_eq!(
            parse_note(&note(&started_note(&"pc-1".to_owned(), "LAPTOP"))),
            Some(Mark::Started { device: Some("pc-1".into()), device_name: Some("LAPTOP".into()) })
        );
        assert_eq!(parse_note(&note("Remember to cite the slides")), None);
        assert_eq!(parse_note(&note("📎 [x](y)\n\nMultiTasker · hologram")), None);
    }

    #[test]
    fn handed_in_files_round_trip_through_comments() {
        let file = HandedInFile {
            id: "n1".into(),
            task_id: "t1".into(),
            name: "Essay final.docx".into(),
            sha256: "ab".repeat(32),
            size: 12_345,
            device: Some("pc-1".into()),
            added_at: Utc::now(),
        };
        let mut n = note(&handed_in_note(&file, "LAPTOP"));
        n.posted_at = file.added_at;
        assert_eq!(parse_note(&n), Some(Mark::HandedIn(file)));
    }

    #[test]
    fn the_label_decides_whether_a_task_is_in_progress() {
        let item = |labels: &[&str]| Item {
            id: "t1".into(),
            content: "Essay".into(),
            description: String::new(),
            project_id: "p".into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
            checked: false,
            is_deleted: false,
            due: None,
            added_at: Utc::now(),
            updated_at: None,
            completed_at: None,
        };
        let notes = [note(&started_note(&"pc-1".to_owned(), "LAPTOP"))];
        let on = extras_of(&item(&["in-progress"]), &notes);
        assert_eq!((on.started_on.as_deref(), on.started_on_name.as_deref()), (Some("pc-1"), Some("LAPTOP")));
        assert!(extras_of(&item(&[]), &notes).started_at.is_none(), "a leftover comment alone doesn't count");
        assert!(extras_of(&item(&["in-progress"]), &[]).started_at.is_some(), "labelled in Todoist itself");
    }

    #[test]
    fn reads_every_kind_of_due_date() {
        let due = |d: &str| due_of(Some(&Due { date: d.into() }));
        assert_eq!(due("2026-11-26"), (NaiveDate::from_ymd_opt(2026, 11, 26), None));
        let (_, at) = due("2026-11-25T08:00:00Z");
        assert_eq!(at, Some(Utc.with_ymd_and_hms(2026, 11, 25, 8, 0, 0).unwrap()));
        let (date, at) = due("2026-11-25T16:00:00");
        assert_eq!(at.map(|t| t.with_timezone(&Local).naive_local()), NaiveDateTime::parse_from_str("2026-11-25T16:00:00", "%Y-%m-%dT%H:%M:%S").ok());
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 11, 25));
        assert_eq!(due("someday"), (None, None));
    }

    #[test]
    fn due_edits_keep_the_day_when_only_the_time_goes() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 26);
        let only_time = TaskChanges { due_at: Some(None), ..Default::default() };
        assert_eq!(due_body(&only_time, day), Some(("due_date", json!("2026-11-26"))));
        let cleared = TaskChanges { due_date: Some(None), ..Default::default() };
        assert_eq!(due_body(&cleared, day), Some(("due_string", json!("no date"))));
        assert_eq!(due_body(&TaskChanges::default(), day), None);
    }

    /// Runs against a real Todoist account: `TODOIST_TOKEN=... cargo test -p multitasker-provider-todoist -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn works_against_a_real_account() {
        let Ok(token) = std::env::var("TODOIST_TOKEN") else { return };
        let todoist = Todoist::new(token);
        assert!(!todoist.account().await.unwrap().is_empty());

        let due = Utc.with_ymd_and_hms(2026, 12, 1, 8, 0, 0).unwrap();
        let task = todoist
            .create(NewTask {
                title: "MultiTasker live test".into(),
                category: Some("MultiTasker Test".into()),
                due_at: Some(due),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!((task.category.as_deref(), task.due_at), (Some("MultiTasker Test"), Some(due)));

        let device = "pc-test".to_owned();
        todoist.start(&task.id, &device, "TEST-PC").await.unwrap();
        todoist.start(&task.id, &device, "TEST-PC").await.unwrap();
        let link = Attachment::new(
            PROVIDER_KEY,
            &task.id,
            &device,
            multitasker_core::NewAttachment::Link { url: "https://example.com/notes".into(), name: Some("Notes".into()) },
        )
        .unwrap();
        let stored = todoist.add_attachment(link).await.unwrap();
        let extras = ExtrasStore::extras(&todoist).await.unwrap().remove(&task.id).unwrap();
        assert_eq!(extras.started_on_name.as_deref(), Some("TEST-PC"));
        assert_eq!(extras.attachments.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [stored.id.as_str()]);

        let edited = todoist
            .update(&task.id, TaskChanges { due_at: Some(None), category: Some(None), ..Default::default() })
            .await
            .unwrap();
        assert_eq!((edited.due_at, edited.due_date, edited.category), (None, Some(due.with_timezone(&Local).date_naive()), None));

        let done = todoist.update(&task.id, TaskChanges { status: Some(TaskStatus::Done), ..Default::default() }).await.unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        let reopened = todoist.update(&task.id, TaskChanges { status: Some(TaskStatus::Todo), ..Default::default() }).await.unwrap();
        assert_eq!(reopened.status, TaskStatus::Todo);
        let extras = ExtrasStore::extras(&todoist).await.unwrap().remove(&task.id).unwrap_or_default();
        assert!(extras.started_at.is_none(), "finishing it stopped it");

        todoist.remove_attachment(&stored.id).await.unwrap();

        // Time adds up across pauses, and the newest left-off note replaces the last.
        todoist.add_worked(&task.id, 90).await.unwrap();
        todoist.add_worked(&task.id, 30).await.unwrap();
        let note = |text: &str| LeftOff { text: text.into(), at: Utc::now(), device: "pc-test".into() };
        todoist.set_left_off(&task.id, Some(note("Intro done"))).await.unwrap();
        todoist.set_left_off(&task.id, Some(note("Halfway through part 2"))).await.unwrap();
        let extras = ExtrasStore::extras(&todoist).await.unwrap().remove(&task.id).unwrap();
        assert_eq!(extras.worked_secs, 120);
        assert_eq!(extras.left_off.map(|n| n.text), Some("Halfway through part 2".into()));
        todoist.set_left_off(&task.id, None).await.unwrap();
        let cleared = ExtrasStore::extras(&todoist).await.unwrap().remove(&task.id).and_then(|e| e.left_off);
        assert_eq!(cleared, None);

        todoist.delete(&task.id).await.unwrap();
        let inner = todoist.inner.clone();
        let project = inner.snapshot().unwrap().projects.into_iter().find(|p| p.name == "MultiTasker Test");
        if let Some(p) = project {
            inner.send("DELETE", &format!("/projects/{}", p.id), None).unwrap();
        }
    }

    async fn hand_in_round_trip(list: &(impl TaskProvider + WorkStore), task: &Task) {
        let bytes = b"final essay".to_vec();
        let file = HandedInFile {
            id: String::new(),
            task_id: task.id.clone(),
            name: "Essay final.txt".into(),
            sha256: "ef".repeat(32),
            size: bytes.len() as u64,
            device: Some("pc-test".into()),
            added_at: Utc::now(),
        };
        let stored = list.hand_in(file.clone(), bytes.clone(), "TEST-PC").await.unwrap();
        assert_eq!((stored.name.as_str(), stored.sha256.as_str(), stored.device.as_deref()), ("Essay final.txt", file.sha256.as_str(), Some("pc-test")));
        let found = WorkStore::handed_in(list, &task.id).await.unwrap();
        assert_eq!(found.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), [stored.id.as_str()]);
        assert_eq!(list.download(&stored).await.unwrap(), bytes);
        assert!(list.all_handed_in().await.unwrap().iter().any(|f| f.id == stored.id));
        list.remove_handed_in(&stored.id).await.unwrap();
        assert!(WorkStore::handed_in(list, &task.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn hands_in_work_to_a_real_account() {
        let Ok(token) = std::env::var("TODOIST_TOKEN") else { return };
        let todoist = Todoist::new(token);
        let task = todoist.create(NewTask { title: "MultiTasker hand-in test".into(), ..Default::default() }).await.unwrap();
        hand_in_round_trip(&todoist, &task).await;
        todoist.delete(&task.id).await.unwrap();
    }
}
