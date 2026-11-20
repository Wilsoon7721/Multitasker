use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use multitasker_core::{check_category, check_title, clean, Error, NewTask, Result, Task, TaskChanges, TaskId, TaskProvider, TaskStatus};
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
    cache: Mutex<Option<(Instant, Vec<Task>)>>,
}

impl JavaDashboard {
    pub fn new(base: impl Into<String>, bearer: Arc<dyn Bearer>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let base = base.into().trim_end_matches('/').to_owned();
        Self { inner: Arc::new(Inner { base, bearer, agent, cache: Mutex::new(None) }) }
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

    fn send(&self, method: &str, url: &str, body: Option<&Value>) -> Result<Value> {
        for attempt in 0..2 {
            let auth = format!("Bearer {}", self.bearer.token().map_err(Error::Storage)?);
            let response = match (method, body) {
                ("GET", _) => self.agent.get(url).header("authorization", &auth).call(),
                ("DELETE", _) => self.agent.delete(url).header("authorization", &auth).call(),
                (m, Some(body)) => {
                    let request = if m == "PATCH" { self.agent.patch(url) } else { self.agent.post(url) };
                    request
                        .header("authorization", &auth)
                        .header("content-type", "application/json")
                        .send(body.to_string().as_bytes())
                }
                (_, None) => self.agent.post(url).header("authorization", &auth).send_empty(),
            };
            let mut response = response.map_err(|e| Error::Storage(format!("Couldn't reach JavaDashboard: {e}")))?;
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
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn list(&self) -> Result<Vec<Task>> {
        if let Some((at, tasks)) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if at.elapsed() < CACHE_FOR {
                return Ok(tasks.clone());
            }
        }
        let body = self.send("GET", &self.url(), None)?;
        let tasks: Vec<Task> = body
            .get("tasks")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value::<RemoteTask>(v.clone()).ok())
            .map(Task::from)
            .collect();
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), tasks.clone()));
        Ok(tasks)
    }

    fn task_from(&self, body: Value) -> Result<Task> {
        self.forget_cache();
        let task = body.get("task").cloned().ok_or_else(|| Error::Storage("JavaDashboard sent no task back.".into()))?;
        serde_json::from_value::<RemoteTask>(task).map(Task::from).map_err(|e| Error::Storage(e.to_string()))
    }
}

/// A task as JavaDashboard's API returns it.
#[derive(Debug, Deserialize)]
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
}

impl From<RemoteTask> for Task {
    fn from(r: RemoteTask) -> Self {
        Task {
            id: r.id,
            title: r.title,
            description: r.description,
            category: r.category,
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

    async fn list(&self) -> Result<Vec<Task>> {
        self.call(|inner| inner.list()).await
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
        let task = Task::from(serde_json::from_value::<RemoteTask>(v).unwrap());
        assert_eq!((task.title.as_str(), task.status), ("GS essay", TaskStatus::Done));
        assert_eq!(task.due_date, NaiveDate::from_ymd_opt(2026, 12, 1));
        assert_eq!((task.due_at, task.category.as_deref()), (None, Some("School")));
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
