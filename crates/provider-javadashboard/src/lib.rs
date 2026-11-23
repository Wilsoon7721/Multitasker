use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use multitasker_core::{
    check_category, check_title, clean, Attachment, AttachmentKind, DeviceId, Error, ExtrasStore, NewTask, Result, Task,
    CheckItem, LeftOff, SettingsStore, SharedSettings, TaskChanges, TaskExtras, TaskId, TaskProvider, TaskStatus,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

pub const PROVIDER_KEY: &str = "javadashboard";

/// A list this fresh is reused, so the UI, detection and reminders don't each ask the server every few seconds.
const CACHE_FOR: Duration = Duration::from_secs(15);

/// Hands out bearer tokens for JavaDashboard's API; the app backs it with a WilsoonID sign-in.
pub trait Bearer: Send + Sync {
    fn token(&self) -> std::result::Result<String, String>;
    /// Forgets the current token after the API rejected it.
    fn invalidate(&self);
}

/// Tasks kept in a JavaDashboard calendar, read and written through its tasks API.
pub struct JavaDashboard {
    inner: Arc<Inner>,
}

struct Inner {
    base: String,
    bearer: Arc<dyn Bearer>,
    agent: ureq::Agent,
    cache: Mutex<Option<(Instant, Vec<RemoteTask>)>>,
    /// Counts changes, so a list read that began before one doesn't put the older list back in the cache.
    changes: std::sync::atomic::AtomicU64,
}

impl JavaDashboard {
    pub fn new(base: impl Into<String>, bearer: Arc<dyn Bearer>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let base = base.into().trim_end_matches('/').to_owned();
        Self { inner: Arc::new(Inner { base, bearer, agent, cache: Mutex::new(None), changes: Default::default() }) }
    }

    /// Runs a blocking API call off the async runtime's worker threads.
    async fn call<T: Send + 'static>(&self, f: impl FnOnce(&Inner) -> Result<T> + Send + 'static) -> Result<T> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || f(&inner)).await.map_err(|e| Error::Storage(e.to_string()))?
    }
}

impl Inner {
    fn url(&self) -> String {
        format!("{}/api/calendar/tasks", self.base)
    }

    fn links_url(&self) -> String {
        format!("{}/api/calendar/tasks/links", self.base)
    }

    fn checklist_url(&self) -> String {
        format!("{}/api/calendar/tasks/checklist", self.base)
    }

    fn settings_url(&self) -> String {
        format!("{}/api/calendar/tasks/settings", self.base)
    }

    fn send(&self, method: &str, url: &str, body: Option<&Value>) -> Result<Value> {
        for attempt in 0..2 {
            let auth = format!("Bearer {}", self.bearer.token().map_err(Error::Storage)?);
            let response = match (method, body) {
                ("GET", _) => self.agent.get(url).header("authorization", &auth).call(),
                ("DELETE", _) => self.agent.delete(url).header("authorization", &auth).call(),
                (m, Some(body)) => {
                    let request = match m {
                        "PATCH" => self.agent.patch(url),
                        "PUT" => self.agent.put(url),
                        _ => self.agent.post(url),
                    };
                    request
                        .header("authorization", &auth)
                        .header("content-type", "application/json")
                        .send(body.to_string().as_bytes())
                }
                (_, None) => self.agent.post(url).header("authorization", &auth).send_empty(),
            };
            let mut response = match response {
                Ok(r) => r,
                // A connection left over from before the computer slept fails once, then a fresh one works. A change
                // that timed out may have arrived anyway, so only reads are tried again after a timeout.
                Err(e) if attempt == 0 && (method == "GET" || !matches!(e, ureq::Error::Timeout(_))) => continue,
                Err(e) => return Err(Error::Storage(format!("Couldn't reach JavaDashboard: {e}"))),
            };
            let status = response.status().as_u16();
            if status == 401 && attempt == 0 {
                self.bearer.invalidate();
                continue;
            }
            let text = response.body_mut().read_to_string().map_err(|e| Error::Storage(e.to_string()))?;
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let message = parsed.get("error").and_then(|e| e.as_str()).map(str::to_owned);
            return match status {
                200..=299 => Ok(parsed),
                401 => Err(Error::Storage("Signed out of JavaDashboard; sign in again in Settings.".into())),
                403 => Err(Error::Invalid(
                    "MultiTasker isn't allowed to do that in JavaDashboard; check its delegation in WilsoonID.".into(),
                )),
                404 => Err(Error::NotFound(message.unwrap_or_else(|| "That task no longer exists.".into()))),
                400..=499 => Err(Error::Invalid(message.unwrap_or_else(|| format!("JavaDashboard refused that ({status}).")))),
                _ => Err(Error::Storage(message.unwrap_or_else(|| format!("JavaDashboard answered {status}.")))),
            };
        }
        unreachable!("the loop returns on its second attempt")
    }

    fn forget_cache(&self) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        self.changes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *cache = None;
    }

    /// Every task as JavaDashboard returns it, extras included; one request serves both the tasks and the extras.
    fn remote(&self) -> Result<Vec<RemoteTask>> {
        if let Some((at, tasks)) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if at.elapsed() < CACHE_FOR {
                return Ok(tasks.clone());
            }
        }
        let before = self.changes.load(std::sync::atomic::Ordering::SeqCst);
        let body = self.send("GET", &self.url(), None)?;
        let tasks: Vec<RemoteTask> = body
            .get("tasks")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value::<RemoteTask>(v.clone()).ok())
            .collect();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if self.changes.load(std::sync::atomic::Ordering::SeqCst) == before {
            *cache = Some((Instant::now(), tasks.clone()));
        }
        Ok(tasks)
    }

    fn task_from(&self, body: Value) -> Result<Task> {
        self.forget_cache();
        let task = body.get("task").cloned().ok_or_else(|| Error::Storage("JavaDashboard sent no task back.".into()))?;
        serde_json::from_value::<RemoteTask>(task).map(|r| Task::from(&r)).map_err(|e| Error::Storage(e.to_string()))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteDevice {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

/// An attachment as JavaDashboard stores it (`calendar.task_links`).
#[derive(Debug, Clone, Deserialize)]
struct RemoteLink {
    id: String,
    kind: String,
    name: String,
    target: String,
    #[serde(default)]
    device: Option<String>,
    added_at: DateTime<Utc>,
}

impl RemoteLink {
    fn into_attachment(self, task_id: &str) -> Option<Attachment> {
        Some(Attachment {
            id: self.id,
            provider: PROVIDER_KEY.into(),
            task_id: task_id.into(),
            kind: AttachmentKind::parse(&self.kind).ok()?,
            name: self.name,
            target: self.target,
            device: self.device,
            added_at: self.added_at,
        })
    }
}

/// A task as JavaDashboard's API returns it.
#[derive(Debug, Clone, Deserialize)]
struct RemoteTask {
    id: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    due_date: Option<NaiveDate>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    started_on: Option<RemoteDevice>,
    #[serde(default)]
    links: Vec<RemoteLink>,
    /// Missing from dashboards that don't keep time yet.
    #[serde(default)]
    worked_seconds: Option<i64>,
    #[serde(default)]
    left_off: Option<RemoteLeftOff>,
    #[serde(default)]
    checklist: Vec<CheckItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteLeftOff {
    text: String,
    at: DateTime<Utc>,
    #[serde(default)]
    device: Option<String>,
}

impl RemoteTask {
    fn extras(&self) -> TaskExtras {
        TaskExtras {
            started_at: self.started_at,
            started_on: self.started_on.as_ref().map(|d| d.id.clone()),
            started_on_name: self.started_on.as_ref().and_then(|d| d.name.clone()),
            attachments: self.links.iter().cloned().filter_map(|l| l.into_attachment(&self.id)).collect(),
            worked_secs: self.worked_seconds.unwrap_or(0),
            left_off: self.left_off.as_ref().map(|n| LeftOff {
                text: n.text.clone(),
                at: n.at,
                device: n.device.clone().unwrap_or_default(),
            }),
            checklist: self.checklist.clone(),
        }
    }
}

impl From<&RemoteTask> for Task {
    fn from(r: &RemoteTask) -> Self {
        Task {
            id: r.id.clone(),
            title: r.title.clone(),
            description: r.description.clone(),
            category: r.category.clone(),
            due_date: r.due_date,
            due_at: None,
            status: TaskStatus::parse(&r.status).unwrap_or(TaskStatus::Todo),
            created_at: r.created_at,
            updated_at: r.updated_at,
            completed_at: r.completed_at,
        }
    }
}

/// The JSON body for an edit; fields JavaDashboard has no place for (a due time) are left out.
fn changes_body(id: &str, changes: TaskChanges) -> Result<Value> {
    let mut body = Map::new();
    body.insert("id".into(), json!(id));
    if let Some(title) = changes.title {
        body.insert("title".into(), json!(check_title(&title)?));
    }
    if let Some(description) = changes.description {
        body.insert("description".into(), json!(clean(description)));
    }
    if let Some(category) = changes.category {
        body.insert("category".into(), json!(check_category(category)?));
    }
    if let Some(due_date) = changes.due_date {
        body.insert("due_date".into(), json!(due_date));
    }
    if let Some(status) = changes.status {
        body.insert("status".into(), json!(status.as_str()));
    }
    Ok(Value::Object(body))
}

#[async_trait]
impl TaskProvider for JavaDashboard {
    fn key(&self) -> &str {
        PROVIDER_KEY
    }

    fn display_name(&self) -> &str {
        "JavaDashboard"
    }

    fn supports_due_time(&self) -> bool {
        false
    }

    fn extras(&self) -> Option<&dyn ExtrasStore> {
        Some(self)
    }

    fn settings(&self) -> Option<&dyn SettingsStore> {
        Some(self)
    }

    async fn list(&self) -> Result<Vec<Task>> {
        self.call(|inner| inner.remote().map(|tasks| tasks.iter().map(Task::from).collect())).await
    }

    async fn create(&self, task: NewTask) -> Result<Task> {
        let body = json!({
            "title": check_title(&task.title)?,
            "description": clean(task.description),
            "category": check_category(task.category)?,
            "due_date": task.due_date.or(task.due_at.map(|at| at.date_naive())),
            "timezone": iana_time_zone::get_timezone().ok(),
        });
        self.call(move |inner| inner.send("POST", &inner.url(), Some(&body)).and_then(|b| inner.task_from(b))).await
    }

    async fn update(&self, id: &TaskId, changes: TaskChanges) -> Result<Task> {
        let body = changes_body(id, changes)?;
        self.call(move |inner| inner.send("PATCH", &inner.url(), Some(&body)).and_then(|b| inner.task_from(b))).await
    }

    async fn delete(&self, id: &TaskId) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            inner.forget_cache();
            inner.send("DELETE", &format!("{}?id={id}", inner.url()), None).map(|_| ())
        })
        .await
    }
}

#[async_trait]
impl ExtrasStore for JavaDashboard {
    async fn extras(&self) -> Result<HashMap<TaskId, TaskExtras>> {
        self.call(|inner| {
            Ok(inner
                .remote()?
                .iter()
                .filter(|t| {
                    t.started_at.is_some()
                        || !t.links.is_empty()
                        || t.worked_seconds.unwrap_or(0) > 0
                        || t.left_off.is_some()
                        || !t.checklist.is_empty()
                })
                .map(|t| (t.id.clone(), t.extras()))
                .collect())
        })
        .await
    }

    async fn start(&self, id: &TaskId, device: &DeviceId, device_name: &str) -> Result<()> {
        let body = json!({ "id": id, "started": true, "device": { "id": device, "name": device_name } });
        self.call(move |inner| inner.send("PATCH", &inner.url(), Some(&body)).map(|_| inner.forget_cache())).await
    }

    async fn stop(&self, id: &TaskId) -> Result<()> {
        let body = json!({ "id": id, "started": false });
        self.call(move |inner| inner.send("PATCH", &inner.url(), Some(&body)).map(|_| inner.forget_cache())).await
    }

    async fn add_attachment(&self, a: Attachment) -> Result<Attachment> {
        let body = json!({
            "task_id": a.task_id,
            "kind": a.kind.as_str(),
            "name": a.name,
            "target": a.target,
            "device": a.device,
        });
        let task_id = a.task_id.clone();
        self.call(move |inner| {
            let response = inner.send("POST", &inner.links_url(), Some(&body))?;
            inner.forget_cache();
            let link = response.get("link").cloned().ok_or_else(|| Error::Storage("JavaDashboard sent no link back.".into()))?;
            serde_json::from_value::<RemoteLink>(link)
                .ok()
                .and_then(|l| l.into_attachment(&task_id))
                .ok_or_else(|| Error::Storage("JavaDashboard sent back a link MultiTasker can't read.".into()))
        })
        .await
    }

    async fn remove_attachment(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |inner| {
            inner.forget_cache();
            inner.send("DELETE", &format!("{}?id={id}", inner.links_url()), None).map(|_| ())
        })
        .await
    }

    async fn add_worked(&self, id: &TaskId, secs: i64) -> Result<()> {
        let body = json!({ "id": id, "add_worked": secs });
        self.call(move |inner| {
            let response = inner.send("PATCH", &inner.url(), Some(&body))?;
            inner.forget_cache();
            kept_by_dashboard(&response, "worked_seconds")
        })
        .await
    }

    async fn set_worked(&self, id: &TaskId, secs: i64) -> Result<()> {
        let body = json!({ "id": id, "worked_seconds": secs });
        self.call(move |inner| {
            let response = inner.send("PATCH", &inner.url(), Some(&body))?;
            inner.forget_cache();
            // A dashboard from before corrections ignores the field, so the total it sends back is checked.
            match response.pointer("/task/worked_seconds").and_then(|v| v.as_i64()) {
                Some(total) if total == secs => Ok(()),
                _ => Err(Error::Invalid("This JavaDashboard can't correct time spent yet.".into())),
            }
        })
        .await
    }

    async fn set_left_off(&self, id: &TaskId, note: Option<LeftOff>) -> Result<()> {
        let left_off = note.map(|n| json!({ "text": n.text, "device": n.device }));
        let body = json!({ "id": id, "left_off": left_off });
        self.call(move |inner| {
            let response = inner.send("PATCH", &inner.url(), Some(&body))?;
            inner.forget_cache();
            kept_by_dashboard(&response, "left_off")
        })
        .await
    }

    async fn add_check(&self, task_id: &TaskId, text: &str) -> Result<CheckItem> {
        let body = json!({ "task_id": task_id, "text": text });
        let response = self.checklist("POST", String::new(), Some(body)).await?;
        serde_json::from_value(response.get("item").cloned().unwrap_or_default())
            .map_err(|_| Error::Storage("JavaDashboard sent back a step MultiTasker can't read.".into()))
    }

    async fn set_check(&self, _task_id: &TaskId, id: &str, done: bool) -> Result<()> {
        self.checklist("PATCH", String::new(), Some(json!({ "id": id, "done": done }))).await.map(|_| ())
    }

    async fn remove_check(&self, _task_id: &TaskId, id: &str) -> Result<()> {
        self.checklist("DELETE", format!("?id={id}"), None).await.map(|_| ())
    }
}

impl JavaDashboard {
    /// Sends one checklist change; a dashboard without checklists yet answers 404 for the address.
    async fn checklist(&self, method: &'static str, query: String, body: Option<Value>) -> Result<Value> {
        self.call(move |inner| {
            let url = format!("{}{query}", inner.checklist_url());
            let response = inner.send(method, &url, body.as_ref());
            inner.forget_cache();
            response
        })
        .await
    }
}

#[async_trait]
impl SettingsStore for JavaDashboard {
    async fn load_settings(&self) -> Result<Option<SharedSettings>> {
        self.call(|inner| {
            let body = inner.send("GET", &inner.settings_url(), None).map_err(older_dashboard)?;
            Ok(body.get("settings").and_then(|s| s.as_str()).map(SharedSettings::from_json))
        })
        .await
    }

    async fn save_settings(&self, settings: &SharedSettings) -> Result<()> {
        let body = json!({ "settings": settings.to_json() });
        self.call(move |inner| inner.send("PUT", &inner.settings_url(), Some(&body)).map(|_| ()).map_err(older_dashboard)).await
    }
}

/// A dashboard from before settings were kept answers 404 for the address.
fn older_dashboard(e: Error) -> Error {
    match e {
        Error::NotFound(_) => Error::Invalid("This JavaDashboard doesn't keep MultiTasker's settings yet.".into()),
        e => e,
    }
}

/// A dashboard from before time and notes were kept takes the change without a word, so its reply is checked for the field.
fn kept_by_dashboard(response: &serde_json::Value, field: &str) -> Result<()> {
    if response.get("task").and_then(|t| t.get(field)).is_some() {
        Ok(())
    } else {
        Err(Error::Invalid(format!("This JavaDashboard doesn't keep {field} yet.")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tasks_as_the_api_returns_them() {
        let v = json!({
            "id": "5f0c", "title": "GS essay", "description": null, "category": "School", "due_date": "2026-12-01",
            "status": "done", "position": 3, "timezone": "Asia/Singapore",
            "created_at": "2026-11-01T02:00:00.000Z", "updated_at": "2026-11-02T02:00:00.000Z",
            "completed_at": "2026-11-02T02:00:00.000Z", "delete_at": null, "days_left": 29, "event_id": null,
            "is_blocked": false, "blocking_reason": null, "blocking_expiry": null
        });
        let task = Task::from(&serde_json::from_value::<RemoteTask>(v).unwrap());
        assert_eq!((task.title.as_str(), task.status), ("GS essay", TaskStatus::Done));
        assert_eq!(task.due_date, NaiveDate::from_ymd_opt(2026, 12, 1));
        assert_eq!((task.due_at, task.category.as_deref()), (None, Some("School")));
    }

    #[test]
    fn reads_in_progress_marks_and_links() {
        let v = json!({
            "id": "t1", "title": "Essay", "status": "todo",
            "created_at": "2026-11-01T02:00:00.000Z", "updated_at": "2026-11-02T02:00:00.000Z",
            "started_at": "2026-11-21T03:00:00.000Z", "started_on": { "id": "pc-1", "name": "LAPTOP" },
            "links": [
                { "id": "l1", "kind": "link", "name": "Claude chat", "target": "https://claude.ai/chat/1", "device": null,
                  "added_at": "2026-11-21T03:05:00.000Z" },
                { "id": "l2", "kind": "hologram", "name": "?", "target": "?", "added_at": "2026-11-21T03:05:00.000Z" }
            ]
        });
        let extras = serde_json::from_value::<RemoteTask>(v).unwrap().extras();
        assert_eq!(extras.started_on.as_deref(), Some("pc-1"));
        assert_eq!(extras.started_on_name.as_deref(), Some("LAPTOP"));
        assert_eq!(extras.attachments.len(), 1, "kinds MultiTasker doesn't know are skipped");
        assert_eq!((extras.attachments[0].task_id.as_str(), extras.attachments[0].provider.as_str()), ("t1", PROVIDER_KEY));
    }

    #[test]
    fn edits_send_only_what_changed_and_drop_due_times() {
        let body = changes_body(
            "t1",
            TaskChanges {
                title: Some(" Final ".into()),
                category: Some(None),
                due_at: Some(Some(Utc::now())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(body, json!({ "id": "t1", "title": "Final", "category": null }));
        assert!(changes_body("t1", TaskChanges { title: Some("  ".into()), ..Default::default() }).is_err());
    }

    struct NoToken;
    impl Bearer for NoToken {
        fn token(&self) -> std::result::Result<String, String> {
            Err("Not signed in.".into())
        }
        fn invalidate(&self) {}
    }

    #[tokio::test]
    async fn surfaces_sign_in_problems() {
        let dash = JavaDashboard::new("https://dash.example.invalid/", Arc::new(NoToken));
        assert_eq!(dash.list().await.unwrap_err().to_string(), "storage error: Not signed in.");
        assert!(!dash.supports_due_time());
    }
}
