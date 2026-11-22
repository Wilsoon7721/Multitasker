use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate, SecondsFormat, Utc};
use multitasker_core::{
    check_category, check_title, clean, content_type, multipart_file, Attachment, AttachmentKind, CheckItem, DeviceId, Error, LeftOff,
    ExtrasStore, HandedInFile, NewTask, Result, SettingsStore, SharedSettings, Task, TaskChanges, TaskExtras, TaskId, TaskProvider, TaskStatus, WorkStore,
};
use serde_json::{json, Map, Value};

pub const PROVIDER_KEY: &str = "notion";

const API: &str = "https://api.notion.com/v1";
const VERSION: &str = "2025-09-03";

/// A list this fresh is reused, so the UI, detection and reminders don't each ask Notion every few seconds.
const CACHE_FOR: Duration = Duration::from_secs(15);

/// Notion caps each piece of rich text at this many characters.
const TEXT_CHUNK: usize = 2000;

/// Marks file paths MultiTasker attached, carrying what kind they are and which computer they're on.
const FRAGMENT: &str = "multitasker=";

/// Ends the caption of every file MultiTasker hands in, so it can find its own in a page.
const MARK: &str = " · MultiTasker · ";

/// The largest file MultiTasker downloads from Notion in one go.
const MAX_FILE: u64 = 200 * 1024 * 1024;

/// A page's handed-in files are looked up again after this long, even if the page seems unchanged.
const RESCAN_AFTER: Duration = Duration::from_secs(15 * 60);

/// At most this many pages are looked through per background check, to stay inside Notion's rate limit.
const SCANS_PER_CHECK: usize = 10;

/// A Notion database shared with the integration, as offered in Settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Database {
    /// The database's data source, which is what tasks are read from and added to.
    pub id: String,
    pub name: String,
}

/// Tasks kept in a Notion database, read and written with an integration token.
pub struct Notion {
    inner: Arc<Inner>,
}

struct Inner {
    token: String,
    source: String,
    agent: ureq::Agent,
    columns: Mutex<Option<Columns>>,
    cache: Mutex<Option<(Instant, Vec<Page>)>>,
    /// Each page's handed-in files, with the page's edit time and when they were read.
    /// What's on each page, read when it last changed: handed-in files and checklist to-dos.
    work: Mutex<HashMap<String, PageScan>>,
}

/// Which of the database's properties hold what.
#[derive(Debug, Clone, PartialEq)]
struct Columns {
    title: String,
    done: Done,
    due: String,
    category: Option<Category>,
    description: String,
    started: String,
    started_on: String,
    files: String,
    /// Time spent, in minutes; a number column people can sum and sort in Notion too.
    minutes: String,
    left_off: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Done {
    /// A Status property: its To-do, In progress and Complete groups, and the option names to set for each.
    Status { property: String, groups: HashMap<String, Group>, todo: String, doing: String, done: String },
    Checkbox(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Todo,
    Doing,
    Done,
}

#[derive(Debug, Clone, PartialEq)]
struct Category {
    property: String,
    multi: bool,
}

/// A page as the query returns it.
#[derive(Debug, Clone)]
struct Page {
    id: String,
    created: DateTime<Utc>,
    edited: DateTime<Utc>,
    properties: Map<String, Value>,
}

/// Quick to give up on a dead connection, patient with a big file on a slow one.
fn new_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_send_body(Some(Duration::from_secs(15 * 60)))
        .timeout_recv_body(Some(Duration::from_secs(15 * 60)))
        .build()
        .into()
}

impl Notion {
    pub fn new(token: impl Into<String>, data_source: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                token: token.into().trim().to_owned(),
                source: data_source.into(),
                agent: new_agent(),
                columns: Mutex::new(None),
                cache: Mutex::new(None),
                work: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Checks a token, returning the workspace's name and the databases shared with the integration.
    pub async fn databases(token: &str) -> Result<(String, Vec<Database>)> {
        let probe = Notion::new(token, "");
        probe
            .call(|inner| {
                let me = inner.send("GET", "/users/me", None)?;
                let workspace = me
                    .pointer("/bot/workspace_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("your Notion workspace")
                    .to_owned();
                let mut found = Vec::new();
                let mut cursor: Option<String> = None;
                for _ in 0..10 {
                    let mut body = json!({ "filter": { "property": "object", "value": "data_source" }, "page_size": 100 });
                    if let Some(c) = &cursor {
                        body["start_cursor"] = json!(c);
                    }
                    let page = inner.send("POST", "/search", Some(&body))?;
                    for r in page.get("results").and_then(|v| v.as_array()).into_iter().flatten() {
                        let Some(id) = r.get("id").and_then(|v| v.as_str()) else { continue };
                        let name = plain(r.get("title")).trim().to_owned();
                        found.push(Database {
                            id: id.to_owned(),
                            name: if name.is_empty() { "Untitled".into() } else { name },
                        });
                    }
                    cursor = page.get("next_cursor").and_then(|v| v.as_str()).map(str::to_owned);
                    if cursor.is_none() {
                        break;
                    }
                }
                Ok((workspace, found))
            })
            .await
    }

    /// Reads the database's layout, adding the columns MultiTasker needs that it doesn't have yet.
    pub async fn prepare(&self) -> Result<()> {
        self.call(|inner| inner.columns().map(|_| ())).await
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
        for attempt in 0..3 {
            let response = match (method, body) {
                ("GET", _) => self.agent.get(&url).header("authorization", &auth).header("notion-version", VERSION).call(),
                ("DELETE", _) => self.agent.delete(&url).header("authorization", &auth).header("notion-version", VERSION).call(),
                (m, Some(body)) => {
                    let request = if m == "PATCH" { self.agent.patch(&url) } else { self.agent.post(&url) };
                    request
                        .header("authorization", &auth)
                        .header("notion-version", VERSION)
                        .header("content-type", "application/json")
                        .send(body.to_string().as_bytes())
                }
                (_, None) => self.agent.post(&url).header("authorization", &auth).header("notion-version", VERSION).send_empty(),
            };
            let mut response = response.map_err(|e| Error::Storage(format!("Couldn't reach Notion: {e}")))?;
            let status = response.status().as_u16();
            // Notion allows about three requests a second and says how long to wait past that.
            if status == 429 && attempt < 2 {
                let wait = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(1)
                    .min(5);
                std::thread::sleep(Duration::from_secs(wait));
                continue;
            }
            let text = response.body_mut().read_to_string().map_err(|e| Error::Storage(e.to_string()))?;
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let message = parsed.get("message").and_then(|e| e.as_str()).map(str::to_owned);
            return match status {
                200..=299 => Ok(parsed),
                401 => Err(Error::Storage("Notion turned the token down; paste a new one in Settings.".into())),
                403 | 404 if path.starts_with("/data_sources") => Err(Error::Storage(
                    "MultiTasker can't see that database any more; share it with your integration again in Notion.".into(),
                )),
                404 => Err(Error::NotFound("That task no longer exists in Notion.".into())),
                429 => Err(Error::Storage("Notion asked MultiTasker to slow down; try again in a minute.".into())),
                400..=499 => Err(Error::Invalid(message.unwrap_or_else(|| format!("Notion refused that ({status}).")))),
                _ => Err(Error::Storage(message.unwrap_or_else(|| format!("Notion answered {status}.")))),
            };
        }
        unreachable!("the loop returns on its last attempt")
    }

    fn columns(&self) -> Result<Columns> {
        if let Some(columns) = self.columns.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(columns);
        }
        let schema = self.send("GET", &format!("/data_sources/{}", self.source), None)?;
        let properties = schema.get("properties").and_then(|v| v.as_object()).cloned().unwrap_or_default();
        let (columns, missing) = plan_columns(&properties)?;
        if !missing.is_empty() {
            self.send("PATCH", &format!("/data_sources/{}", self.source), Some(&json!({ "properties": missing })))?;
        }
        *self.columns.lock().unwrap_or_else(|e| e.into_inner()) = Some(columns.clone());
        Ok(columns)
    }

    fn forget_cache(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn pages(&self) -> Result<Vec<Page>> {
        if let Some((at, pages)) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if at.elapsed() < CACHE_FOR {
                return Ok(pages.clone());
            }
        }
        let mut pages = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..10 {
            let mut body = json!({ "page_size": 100 });
            if let Some(c) = &cursor {
                body["start_cursor"] = json!(c);
            }
            let result = self.send("POST", &format!("/data_sources/{}/query", self.source), Some(&body))?;
            pages.extend(
                result.get("results").and_then(|v| v.as_array()).into_iter().flatten().filter_map(page_of).filter(|p| !is_settings(p)),
            );
            cursor = result.get("next_cursor").and_then(|v| v.as_str()).map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), pages.clone()));
        Ok(pages)
    }

    fn page(&self, id: &str) -> Result<Page> {
        page_of(&self.send("GET", &format!("/pages/{id}"), None)?)
            .ok_or_else(|| Error::Storage("Notion sent back a page MultiTasker can't read.".into()))
    }

    fn set(&self, id: &str, properties: Map<String, Value>) -> Result<Page> {
        let page = self.send("PATCH", &format!("/pages/{id}"), Some(&json!({ "properties": properties })))?;
        self.forget_cache();
        page_of(&page).ok_or_else(|| Error::Storage("Notion sent back a page MultiTasker can't read.".into()))
    }
}

fn page_of(v: &Value) -> Option<Page> {
    if v.get("in_trash").and_then(|t| t.as_bool()).unwrap_or(false) {
        return None;
    }
    let time = |key: &str| v.get(key).and_then(|t| t.as_str()).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc));
    Some(Page {
        id: v.get("id")?.as_str()?.to_owned(),
        created: time("created_time")?,
        edited: time("last_edited_time")?,
        properties: v.get("properties")?.as_object()?.clone(),
    })
}

/// The page MultiTasker keeps its shared settings on, left out of the tasks.
const SETTINGS_TITLE: &str = "⚙ MultiTasker settings";

fn is_settings(page: &Page) -> bool {
    page.properties
        .values()
        .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("title"))
        .is_some_and(|v| plain(v.get("title")).trim() == SETTINGS_TITLE)
}

/// Concatenated text of a rich-text or title array.
fn plain(v: Option<&Value>) -> String {
    v.and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
        .collect()
}

fn rich(text: &str) -> Value {
    let chars: Vec<char> = text.chars().collect();
    Value::Array(chars.chunks(TEXT_CHUNK).map(|c| json!({ "text": { "content": c.iter().collect::<String>() } })).collect())
}

fn find<'a>(properties: &'a Map<String, Value>, kind: &str, names: &[&str]) -> Option<&'a str> {
    let of_kind = || properties.iter().filter(move |(_, v)| v.get("type").and_then(|t| t.as_str()) == Some(kind));
    of_kind()
        .find(|(k, _)| names.iter().any(|n| k.eq_ignore_ascii_case(n)))
        .or_else(|| of_kind().find(|(k, _)| !names.is_empty() && names.iter().any(|n| k.to_lowercase().contains(&n.to_lowercase()))))
        .map(|(k, _)| k.as_str())
}

/// Matches the database's properties to what MultiTasker needs, listing the ones to add.
fn plan_columns(properties: &Map<String, Value>) -> Result<(Columns, Map<String, Value>)> {
    let title = find(properties, "title", &[]).or_else(|| {
        properties.iter().find(|(_, v)| v.get("type").and_then(|t| t.as_str()) == Some("title")).map(|(k, _)| k.as_str())
    });
    let title = title.ok_or_else(|| Error::Invalid("That database has no title column.".into()))?.to_owned();
    let mut missing = Map::new();
    let mut pick = |kind: &str, names: &[&str], create: &str, any: bool| -> String {
        let found = find(properties, kind, names).or_else(|| {
            any.then(|| {
                properties
                    .iter()
                    .find(|(k, v)| {
                        v.get("type").and_then(|t| t.as_str()) == Some(kind) && !["Started", "Started on"].contains(&k.as_str())
                    })
                    .map(|(k, _)| k.as_str())
            })
            .flatten()
        });
        match found {
            Some(name) => name.to_owned(),
            None => {
                missing.insert(create.into(), json!({ kind: {} }));
                create.to_owned()
            }
        }
    };

    let done = match status_column(properties) {
        Some(done) => done,
        None => Done::Checkbox(pick("checkbox", &["Done", "Completed", "Complete"], "Done", false)),
    };
    let due = pick("date", &["Due", "Due date", "Deadline", "Date"], "Due", true);
    let description = pick("rich_text", &["Description", "Notes", "Details"], "Description", false);
    let started = pick("date", &["Started"], "Started", false);
    let started_on = pick("rich_text", &["Started on"], "Started on", false);
    let files = pick("files", &["Attachments", "Files", "Files & media"], "Attachments", true);
    let minutes = pick("number", &["Minutes spent"], "Minutes spent", false);
    let left_off = pick("rich_text", &["Left off"], "Left off", false);
    let category = match find(properties, "select", &["Category", "Project", "Type", "Area"]) {
        Some(name) => Category { property: name.to_owned(), multi: false },
        None => match find(properties, "multi_select", &["Category", "Tags", "Project", "Area"]) {
            Some(name) => Category { property: name.to_owned(), multi: true },
            None => {
                missing.insert("Category".into(), json!({ "select": {} }));
                Category { property: "Category".into(), multi: false }
            }
        },
    };
    let columns = Columns { title, done, due, category: Some(category), description, started, started_on, files, minutes, left_off };
    Ok((columns, missing))
}

/// A Status property, with the option to set for each of its three groups.
fn status_column(properties: &Map<String, Value>) -> Option<Done> {
    let name = find(properties, "status", &["Status"])?;
    let status = properties.get(name)?.get("status")?;
    let options: HashMap<&str, &str> = status
        .get("options")?
        .as_array()?
        .iter()
        .filter_map(|o| Some((o.get("id")?.as_str()?, o.get("name")?.as_str()?)))
        .collect();
    let mut groups = HashMap::new();
    let mut first = [None, None, None];
    for (i, g) in status.get("groups")?.as_array()?.iter().enumerate() {
        let label = g.get("name").and_then(|n| n.as_str()).unwrap_or("").to_lowercase();
        let group = match label.as_str() {
            "to-do" | "to do" | "todo" | "not started" => Group::Todo,
            "in progress" | "doing" => Group::Doing,
            "complete" | "completed" | "done" => Group::Done,
            _ => [Group::Todo, Group::Doing, Group::Done][i.min(2)],
        };
        for id in g.get("option_ids").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str()) {
            if let Some(option) = options.get(id) {
                groups.insert(option.to_string(), group);
                let slot = &mut first[group as usize];
                if slot.is_none() {
                    *slot = Some(option.to_string());
                }
            }
        }
    }
    let [todo, doing, done] = first;
    Some(Done::Status { property: name.to_owned(), groups, todo: todo?, doing: doing?, done: done? })
}

impl Columns {
    fn group(&self, page: &Page) -> Group {
        match &self.done {
            Done::Status { property, groups, .. } => page
                .properties
                .get(property)
                .and_then(|p| p.pointer("/status/name"))
                .and_then(|n| n.as_str())
                .and_then(|n| groups.get(n).copied())
                .unwrap_or(Group::Todo),
            Done::Checkbox(property) => {
                if page.properties.get(property).and_then(|p| p.get("checkbox")).and_then(|c| c.as_bool()).unwrap_or(false) {
                    Group::Done
                } else if self.started_at(page).is_some() {
                    Group::Doing
                } else {
                    Group::Todo
                }
            }
        }
    }

    fn group_value(&self, group: Group) -> (String, Value) {
        match &self.done {
            Done::Status { property, todo, doing, done, .. } => {
                let option = match group {
                    Group::Todo => todo,
                    Group::Doing => doing,
                    Group::Done => done,
                };
                (property.clone(), json!({ "status": { "name": option } }))
            }
            Done::Checkbox(property) => (property.clone(), json!({ "checkbox": group == Group::Done })),
        }
    }

    fn started_at(&self, page: &Page) -> Option<DateTime<Utc>> {
        let start = page.properties.get(&self.started)?.pointer("/date/start")?.as_str()?;
        DateTime::parse_from_rfc3339(start).ok().map(|t| t.with_timezone(&Utc))
    }

    fn due(&self, page: &Page) -> (Option<NaiveDate>, Option<DateTime<Utc>>) {
        let raw = page.properties.get(&self.due).and_then(|p| p.pointer("/date/start")).and_then(|s| s.as_str());
        due_of(raw)
    }

    fn task(&self, page: &Page) -> Task {
        let (due_date, due_at) = self.due(page);
        let category = self.category.as_ref().and_then(|c| {
            let p = page.properties.get(&c.property)?;
            let name = if c.multi { p.pointer("/multi_select/0/name") } else { p.pointer("/select/name") };
            name.and_then(|n| n.as_str()).map(str::to_owned)
        });
        let done = self.group(page) == Group::Done;
        Task {
            id: page.id.clone(),
            title: plain(page.properties.get(&self.title).and_then(|p| p.get("title"))).trim().to_owned(),
            description: clean(Some(plain(page.properties.get(&self.description).and_then(|p| p.get("rich_text"))))),
            category,
            due_date,
            due_at,
            status: if done { TaskStatus::Done } else { TaskStatus::Todo },
            created_at: page.created,
            updated_at: page.edited,
            completed_at: done.then_some(page.edited),
        }
    }

    fn extras(&self, page: &Page) -> TaskExtras {
        let attachments = page
            .properties
            .get(&self.files)
            .and_then(|p| p.get("files"))
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
            .filter_map(|f| file_attachment(&page.id, f, page.edited))
            .collect();
        let worked_secs = self.worked_secs(page);
        let left_off = clean(Some(plain(page.properties.get(&self.left_off).and_then(|p| p.get("rich_text")))))
            .map(|text| LeftOff { text, at: page.edited, device: String::new() });
        if self.group(page) != Group::Doing {
            return TaskExtras { attachments, worked_secs, left_off, ..Default::default() };
        }
        let name = plain(page.properties.get(&self.started_on).and_then(|p| p.get("rich_text")));
        TaskExtras {
            // Moved to In progress in Notion itself, it has no start time of its own yet.
            started_at: Some(self.started_at(page).unwrap_or(page.edited)),
            started_on: None,
            started_on_name: clean(Some(name)),
            attachments,
            worked_secs,
            left_off,
            checklist: Vec::new(),
        }
    }

    fn worked_secs(&self, page: &Page) -> i64 {
        let minutes = page.properties.get(&self.minutes).and_then(|p| p.get("number")).and_then(|n| n.as_f64()).unwrap_or(0.0);
        (minutes * 60.0).round().max(0.0) as i64
    }

    /// Property values for a new task or an edit.
    fn values(&self, changes: &TaskChanges, current: Option<NaiveDate>) -> Result<Map<String, Value>> {
        let mut out = Map::new();
        if let Some(title) = &changes.title {
            out.insert(self.title.clone(), json!({ "title": rich(&check_title(title)?) }));
        }
        if let Some(description) = &changes.description {
            out.insert(self.description.clone(), json!({ "rich_text": rich(&clean(description.clone()).unwrap_or_default()) }));
        }
        if let (Some(category), Some(column)) = (&changes.category, &self.category) {
            let value = check_category(category.clone())?;
            let value = match (column.multi, value) {
                (true, v) => json!({ "multi_select": v.map(|n| vec![json!({ "name": n.replace(',', " ") })]).unwrap_or_default() }),
                (false, Some(n)) => json!({ "select": { "name": n.replace(',', " ") } }),
                (false, None) => json!({ "select": null }),
            };
            out.insert(column.property.clone(), value);
        }
        let date = match (changes.due_at, changes.due_date) {
            (Some(Some(at)), _) => Some(json!({ "start": at.to_rfc3339_opts(SecondsFormat::Secs, true) })),
            (_, Some(Some(day))) => Some(json!({ "start": day })),
            (_, Some(None)) => Some(Value::Null),
            // Taking the time off keeps the day.
            (Some(None), None) => Some(current.map(|d| json!({ "start": d })).unwrap_or(Value::Null)),
            (None, None) => None,
        };
        if let Some(date) = date {
            out.insert(self.due.clone(), json!({ "date": date }));
        }
        Ok(out)
    }

    fn clear_start(&self, out: &mut Map<String, Value>) {
        out.insert(self.started.clone(), json!({ "date": null }));
        out.insert(self.started_on.clone(), json!({ "rich_text": [] }));
    }
}

/// Notion dates come as a plain date or a moment with its offset.
fn due_of(raw: Option<&str>) -> (Option<NaiveDate>, Option<DateTime<Utc>>) {
    let Some(raw) = raw else { return (None, None) };
    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return (Some(date), None);
    }
    match DateTime::parse_from_rfc3339(raw) {
        Ok(at) => {
            let at = at.with_timezone(&Utc);
            (Some(at.with_timezone(&Local).date_naive()), Some(at))
        }
        Err(_) => (None, None),
    }
}

fn guess_kind(name: &str) -> AttachmentKind {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => AttachmentKind::Pdf,
        "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "xls" | "xlsx" | "ods" | "csv" | "ppt" | "pptx" | "odp" => {
            AttachmentKind::Document
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "heic" | "avif" | "tif" | "tiff" => AttachmentKind::Image,
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "wmv" | "m4v" => AttachmentKind::Video,
        "exe" => AttachmentKind::App,
        _ => AttachmentKind::File,
    }
}

/// A Files & media entry as an attachment; ids carry the page so any task's attachment can be found again.
fn file_attachment(page: &str, f: &Value, edited: DateTime<Utc>) -> Option<Attachment> {
    let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("").to_owned();
    let (url, hosted) = match f.get("type").and_then(|t| t.as_str())? {
        "external" => (f.pointer("/external/url")?.as_str()?.to_owned(), false),
        "file" => (f.pointer("/file/url")?.as_str()?.to_owned(), true),
        _ => return None,
    };
    // Files uploaded to Notion get a fresh link every hour, so they're known by name instead.
    let key = if hosted { format!("upload:{name}") } else { url.clone() };
    let id = format!("{page}|{key}");
    if let Some(rest) = url.strip_prefix("file:///") {
        let (path, fragment) = rest.split_once('#').unwrap_or((rest, ""));
        let path = percent_decode(path).replace('/', "\\");
        let (kind, device) = match fragment.strip_prefix(FRAGMENT) {
            Some(meta) => {
                let (kind, device) = meta.split_once(',').unwrap_or((meta, ""));
                (AttachmentKind::parse(kind).unwrap_or_else(|_| guess_kind(&path)), Some(device.to_owned()).filter(|d| !d.is_empty()))
            }
            None => (guess_kind(&path), None),
        };
        let name = if name.is_empty() { path.rsplit('\\').next().unwrap_or(&path).to_owned() } else { name };
        return Some(Attachment { id, provider: PROVIDER_KEY.into(), task_id: page.into(), kind, name, target: path, device, added_at: edited });
    }
    let name = if name.is_empty() { url.clone() } else { name };
    Some(Attachment { id, provider: PROVIDER_KEY.into(), task_id: page.into(), kind: AttachmentKind::Link, name, target: url, device: None, added_at: edited })
}

/// An attachment as a Files & media entry; local paths become file links that remember their kind and computer.
fn file_entry(a: &Attachment) -> Value {
    let name: String = a.name.chars().take(100).collect();
    let url = if a.kind == AttachmentKind::Link {
        a.target.clone()
    } else {
        format!("file:///{}#{FRAGMENT}{},{}", percent_encode(&a.target.replace('\\', "/")), a.kind.as_str(), a.device.as_deref().unwrap_or(""))
    };
    json!({ "name": name, "type": "external", "external": { "url": url } })
}

fn percent_encode(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[async_trait]
impl TaskProvider for Notion {
    fn key(&self) -> &str {
        PROVIDER_KEY
    }

    fn display_name(&self) -> &str {
        "Notion"
    }

    fn extras(&self) -> Option<&dyn ExtrasStore> {
        Some(self)
    }

    fn work(&self) -> Option<&dyn WorkStore> {
        Some(self)
    }

    fn settings(&self) -> Option<&dyn SettingsStore> {
        Some(self)
    }

    async fn list(&self) -> Result<Vec<Task>> {
        self.call(|inner| {
            let columns = inner.columns()?;
            Ok(inner.pages()?.iter().map(|p| columns.task(p)).collect())
        })
        .await
    }

    async fn create(&self, task: NewTask) -> Result<Task> {
        self.call(move |inner| {
            let columns = inner.columns()?;
            let changes = TaskChanges {
                title: Some(task.title),
                description: task.description.map(Some),
                category: Some(task.category),
                due_date: task.due_date.map(Some),
                due_at: task.due_at.map(Some),
                ..Default::default()
            };
            let mut properties = columns.values(&changes, None)?;
            let (key, value) = columns.group_value(Group::Todo);
            properties.insert(key, value);
            let body = json!({ "parent": { "type": "data_source_id", "data_source_id": inner.source }, "properties": properties });
            let page = inner.send("POST", "/pages", Some(&body))?;
            inner.forget_cache();
            page_of(&page).map(|p| columns.task(&p)).ok_or_else(|| Error::Storage("Notion sent no task back.".into()))
        })
        .await
    }

    async fn update(&self, id: &TaskId, changes: TaskChanges) -> Result<Task> {
        let id = id.clone();
        self.call(move |inner| {
            let columns = inner.columns()?;
            let current = inner.page(&id)?;
            let mut properties = columns.values(&changes, columns.due(&current).0)?;
            let group = columns.group(&current);
            match changes.status {
                // A finished task is no longer in progress anywhere.
                Some(TaskStatus::Done) if group != Group::Done => {
                    let (key, value) = columns.group_value(Group::Done);
                    properties.insert(key, value);
                    columns.clear_start(&mut properties);
                }
                Some(TaskStatus::Todo) if group == Group::Done => {
                    let (key, value) = columns.group_value(Group::Todo);
                    properties.insert(key, value);
                }
                _ => {}
            }
            if properties.is_empty() {
                return Ok(columns.task(&current));
            }
            inner.set(&id, properties).map(|p| columns.task(&p))
        })
        .await
    }

    async fn delete(&self, id: &TaskId) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            inner.send("PATCH", &format!("/pages/{id}"), Some(&json!({ "in_trash": true })))?;
            inner.forget_cache();
            Ok(())
        })
        .await
    }
}

#[async_trait]
impl ExtrasStore for Notion {
    async fn extras(&self) -> Result<HashMap<TaskId, TaskExtras>> {
        self.call(|inner| {
            let columns = inner.columns()?;
            let pages = inner.pages()?;
            inner.rescan(&pages, |p| columns.group(p) == Group::Doing)?;
            let work = inner.work.lock().unwrap_or_else(|e| e.into_inner()).clone();
            Ok(pages
                .iter()
                .map(|p| {
                    let mut extras = columns.extras(p);
                    extras.checklist = work.get(&p.id).map(|scan| scan.checks.clone()).unwrap_or_default();
                    (p.id.clone(), extras)
                })
                .filter(|(_, e)| {
                    e.started_at.is_some()
                        || !e.attachments.is_empty()
                        || e.worked_secs > 0
                        || e.left_off.is_some()
                        || !e.checklist.is_empty()
                })
                .collect())
        })
        .await
    }

    async fn start(&self, id: &TaskId, _device: &DeviceId, device_name: &str) -> Result<()> {
        let (id, device_name) = (id.clone(), device_name.to_owned());
        self.call(move |inner| {
            let columns = inner.columns()?;
            let current = inner.page(&id)?;
            let mut properties = Map::new();
            if columns.group(&current) != Group::Doing || matches!(columns.done, Done::Checkbox(_)) {
                let (key, value) = columns.group_value(Group::Doing);
                properties.insert(key, value);
            }
            // Starting again keeps when and where it was first started.
            if columns.started_at(&current).is_none() {
                properties.insert(columns.started.clone(), json!({ "date": { "start": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true) } }));
                properties.insert(columns.started_on.clone(), json!({ "rich_text": rich(&device_name) }));
            }
            if !properties.is_empty() {
                inner.set(&id, properties)?;
            }
            Ok(())
        })
        .await
    }

    async fn stop(&self, id: &TaskId) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            let columns = inner.columns()?;
            let current = inner.page(&id)?;
            let mut properties = Map::new();
            if columns.group(&current) == Group::Doing {
                let (key, value) = columns.group_value(Group::Todo);
                properties.insert(key, value);
            }
            columns.clear_start(&mut properties);
            inner.set(&id, properties).map(|_| ())
        })
        .await
    }

    async fn add_check(&self, task_id: &TaskId, text: &str) -> Result<CheckItem> {
        let (task_id, text) = (task_id.clone(), text.to_owned());
        self.call(move |inner| {
            let block = json!({ "object": "block", "type": "to_do", "to_do": { "rich_text": rich(&text), "checked": false } });
            let result = inner.send("PATCH", &format!("/blocks/{task_id}/children"), Some(&json!({ "children": [block] })))?;
            inner.forget_page(&task_id);
            inner.forget_cache();
            result
                .get("results")
                .and_then(|r| r.as_array())
                .and_then(|r| r.iter().rev().find_map(check_block))
                .ok_or_else(|| Error::Storage("Notion didn't send the new to-do back.".into()))
        })
        .await
    }

    async fn set_check(&self, task_id: &TaskId, id: &str, done: bool) -> Result<()> {
        let (task_id, id) = (task_id.clone(), id.to_owned());
        self.call(move |inner| {
            inner.send("PATCH", &format!("/blocks/{id}"), Some(&json!({ "to_do": { "checked": done } })))?;
            inner.forget_page(&task_id);
            inner.forget_cache();
            Ok(())
        })
        .await
    }

    async fn remove_check(&self, task_id: &TaskId, id: &str) -> Result<()> {
        let (task_id, id) = (task_id.clone(), id.to_owned());
        self.call(move |inner| {
            inner.send("DELETE", &format!("/blocks/{id}"), None)?;
            inner.forget_page(&task_id);
            inner.forget_cache();
            Ok(())
        })
        .await
    }

    async fn add_worked(&self, id: &TaskId, secs: i64) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            let columns = inner.columns()?;
            // Read fresh, so time another computer just added isn't overwritten.
            let total = columns.worked_secs(&inner.page(&id)?) + secs;
            let minutes = (total as f64 / 60.0 * 100.0).round() / 100.0;
            let mut properties = Map::new();
            properties.insert(columns.minutes.clone(), json!({ "number": minutes }));
            inner.set(&id, properties).map(|_| ())
        })
        .await
    }

    async fn set_left_off(&self, id: &TaskId, note: Option<LeftOff>) -> Result<()> {
        let id = id.clone();
        self.call(move |inner| {
            let columns = inner.columns()?;
            let mut properties = Map::new();
            let text = note.map(|n| n.text).unwrap_or_default();
            properties.insert(columns.left_off.clone(), json!({ "rich_text": rich(&text) }));
            inner.set(&id, properties).map(|_| ())
        })
        .await
    }

    async fn add_attachment(&self, a: Attachment) -> Result<Attachment> {
        self.call(move |inner| {
            let columns = inner.columns()?;
            let current = inner.page(&a.task_id)?;
            let mut files: Vec<Value> = current
                .properties
                .get(&columns.files)
                .and_then(|p| p.get("files"))
                .and_then(|f| f.as_array())
                .cloned()
                .unwrap_or_default();
            let entry = file_entry(&a);
            let wanted = file_attachment(&a.task_id, &entry, current.edited).map(|x| x.id);
            if !files.iter().any(|f| file_attachment(&a.task_id, f, current.edited).map(|x| x.id) == wanted) {
                files.push(entry);
            }
            let mut properties = Map::new();
            properties.insert(columns.files.clone(), json!({ "files": files }));
            let page = inner.set(&a.task_id, properties)?;
            columns
                .extras(&page)
                .attachments
                .into_iter()
                .find(|x| Some(&x.id) == wanted.as_ref())
                .ok_or_else(|| Error::Storage("Notion changed the attachment so MultiTasker can't read it back.".into()))
        })
        .await
    }

    async fn remove_attachment(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |inner| {
            let (page_id, _) = id.split_once('|').ok_or_else(|| Error::NotFound("That attachment no longer exists.".into()))?;
            let columns = inner.columns()?;
            let current = inner.page(page_id)?;
            let files: Vec<Value> = current
                .properties
                .get(&columns.files)
                .and_then(|p| p.get("files"))
                .and_then(|f| f.as_array())
                .into_iter()
                .flatten()
                .filter(|f| file_attachment(page_id, f, current.edited).is_none_or(|x| x.id != id))
                .cloned()
                .collect();
            let mut properties = Map::new();
            properties.insert(columns.files.clone(), json!({ "files": files }));
            inner.set(page_id, properties).map(|_| ())
        })
        .await
    }
}

fn handed_in_caption(file: &HandedInFile, device_name: &str) -> String {
    format!(
        "Handed in from {}{MARK}{} · {} · {} · {}",
        device_name.split_whitespace().collect::<Vec<_>>().join(" "),
        file.sha256,
        file.size,
        file.device.as_deref().unwrap_or("-"),
        file.added_at.to_rfc3339_opts(SecondsFormat::Millis, true)
    )
}

/// A file block MultiTasker added to a task page, read back.
fn handed_in_block(task_id: &str, block: &Value) -> Option<HandedInFile> {
    if block.get("type")?.as_str()? != "file" || block.get("in_trash").and_then(|t| t.as_bool()).unwrap_or(false) {
        return None;
    }
    let file = block.get("file")?;
    let caption = plain(file.get("caption"));
    let (_, meta) = caption.split_once(MARK)?;
    let parts: Vec<&str> = meta.split(" · ").collect();
    let added_at = parts
        .get(3)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .or_else(|| block.get("created_time")?.as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc)))?;
    Some(HandedInFile {
        id: block.get("id")?.as_str()?.to_owned(),
        task_id: task_id.to_owned(),
        name: file.get("name").and_then(|n| n.as_str()).unwrap_or("Handed-in file").to_owned(),
        sha256: parts.first()?.to_string(),
        size: parts.get(1)?.parse().ok()?,
        device: parts.get(2).filter(|d| **d != "-").map(|d| d.to_string()),
        added_at,
    })
}

impl Inner {
    fn settings_page(&self) -> Result<Option<String>> {
        let columns = self.columns()?;
        let body = json!({ "filter": { "property": columns.title, "title": { "equals": SETTINGS_TITLE } }, "page_size": 1 });
        let result = self.send("POST", &format!("/data_sources/{}/query", self.source), Some(&body))?;
        Ok(result.get("results").and_then(|v| v.as_array()).and_then(|r| r.first()).and_then(|p| p.get("id")?.as_str()).map(str::to_owned))
    }

    /// The code block holding the settings, and what it says.
    fn settings_block(&self, page: &str) -> Result<Option<(String, SharedSettings)>> {
        let result = self.send("GET", &format!("/blocks/{page}/children?page_size=100"), None)?;
        Ok(result.get("results").and_then(|v| v.as_array()).into_iter().flatten().find_map(|b| {
            let code = b.get("code")?;
            Some((b.get("id")?.as_str()?.to_owned(), SharedSettings::from_json(&plain(code.get("rich_text")))))
        }))
    }
}

fn settings_code(settings: &SharedSettings) -> Value {
    json!({ "rich_text": rich(&settings.to_json()), "language": "json" })
}

#[async_trait]
impl SettingsStore for Notion {
    async fn load_settings(&self) -> Result<Option<SharedSettings>> {
        self.call(|inner| match inner.settings_page()? {
            Some(page) => Ok(inner.settings_block(&page)?.map(|(_, s)| s)),
            None => Ok(None),
        })
        .await
    }

    async fn save_settings(&self, settings: &SharedSettings) -> Result<()> {
        let code = settings_code(settings);
        self.call(move |inner| {
            let block = json!({ "object": "block", "type": "code", "code": code });
            let Some(page) = inner.settings_page()? else {
                // Filed as done, so it stays out of the way of the database's open tasks in Notion too.
                let columns = inner.columns()?;
                let mut properties = Map::new();
                properties.insert(columns.title.clone(), json!({ "title": rich(SETTINGS_TITLE) }));
                let (key, value) = columns.group_value(Group::Done);
                properties.insert(key, value);
                let body = json!({
                    "parent": { "type": "data_source_id", "data_source_id": inner.source },
                    "properties": properties,
                    "children": [block],
                });
                inner.send("POST", "/pages", Some(&body))?;
                return Ok(());
            };
            match inner.settings_block(&page)? {
                Some((id, _)) => inner.send("PATCH", &format!("/blocks/{id}"), Some(&json!({ "code": block["code"] })))?,
                None => inner.send("PATCH", &format!("/blocks/{page}/children"), Some(&json!({ "children": [block] })))?,
            };
            Ok(())
        })
        .await
    }
}

/// A page's blocks as last read, and when, so it's only read again once it changes.
#[derive(Debug, Clone)]
struct PageScan {
    edited: DateTime<Utc>,
    read: Instant,
    files: Vec<HandedInFile>,
    checks: Vec<CheckItem>,
}

/// A to-do block as a checklist step.
fn check_block(block: &Value) -> Option<CheckItem> {
    if block.get("type")?.as_str()? != "to_do" {
        return None;
    }
    let todo = block.get("to_do")?;
    Some(CheckItem {
        id: block.get("id")?.as_str()?.to_owned(),
        text: plain(todo.get("rich_text")),
        done: todo.get("checked").and_then(|c| c.as_bool()).unwrap_or(false),
        added_at: block
            .get("created_time")
            .and_then(|t| t.as_str())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
    })
}

impl Inner {
    /// What's on one page, read fresh: the files MultiTasker handed in, and its to-dos.
    fn page_blocks(&self, page: &str) -> Result<(Vec<HandedInFile>, Vec<CheckItem>)> {
        let (mut files, mut checks) = (Vec::new(), Vec::new());
        let mut cursor: Option<String> = None;
        for _ in 0..10 {
            let mut path = format!("/blocks/{page}/children?page_size=100");
            if let Some(c) = &cursor {
                path.push_str(&format!("&start_cursor={c}"));
            }
            let result = self.send("GET", &path, None)?;
            for block in result.get("results").and_then(|v| v.as_array()).into_iter().flatten() {
                files.extend(handed_in_block(page, block));
                checks.extend(check_block(block));
            }
            cursor = result.get("next_cursor").and_then(|v| v.as_str()).map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok((files, checks))
    }

    fn remember(&self, page: &str, edited: DateTime<Utc>, files: Vec<HandedInFile>, checks: Vec<CheckItem>) {
        let scan = PageScan { edited, read: Instant::now(), files, checks };
        self.work.lock().unwrap_or_else(|e| e.into_inner()).insert(page.to_owned(), scan);
    }

    /// Reads again the pages that changed since they were last read, a few per call so a big database fills in
    /// gradually; pages in progress come first.
    fn rescan(&self, pages: &[Page], first: impl Fn(&Page) -> bool) -> Result<()> {
        let mut order: Vec<&Page> = pages.iter().collect();
        order.sort_by_key(|p| (!first(p), std::cmp::Reverse(p.edited)));
        let mut scanned = 0;
        for page in order {
            let stale = match self.work.lock().unwrap_or_else(|e| e.into_inner()).get(&page.id) {
                Some(scan) => scan.edited != page.edited || scan.read.elapsed() > RESCAN_AFTER,
                None => true,
            };
            if stale && scanned < SCANS_PER_CHECK {
                scanned += 1;
                let (files, checks) = self.page_blocks(&page.id)?;
                self.remember(&page.id, page.edited, files, checks);
            }
        }
        Ok(())
    }

    fn forget_page(&self, page: &str) {
        self.work.lock().unwrap_or_else(|e| e.into_inner()).remove(page);
    }

    /// The largest file this workspace takes, as Notion reports it.
    fn upload_limit(&self) -> Option<u64> {
        self.send("GET", "/users/me", None).ok()?.pointer("/bot/workspace_limits/max_file_upload_size_in_bytes")?.as_u64()
    }

    /// Uploads a file in Notion's two steps: ask for an upload, then send the content to it.
    fn upload(&self, name: &str, bytes: &[u8]) -> Result<String> {
        if let Some(limit) = self.upload_limit().filter(|l| (bytes.len() as u64) > *l) {
            return Err(Error::Invalid(format!(
                "{name} is {:.1} MB; this Notion workspace takes files up to {} MB.",
                bytes.len() as f64 / 1_048_576.0,
                limit / 1_048_576
            )));
        }
        let kind = content_type(name);
        let created = self.send("POST", "/file_uploads", Some(&json!({ "mode": "single_part", "filename": name, "content_type": kind })))?;
        let id = created.get("id").and_then(|v| v.as_str()).ok_or_else(|| Error::Storage("Notion didn't start the upload.".into()))?.to_owned();
        let (body, form) = multipart_file("file", name, kind, bytes);
        let mut response = self
            .agent
            .post(&format!("{API}/file_uploads/{id}/send"))
            .header("authorization", &format!("Bearer {}", self.token))
            .header("notion-version", VERSION)
            .header("content-type", &form)
            .send(&body[..])
            .map_err(|e| Error::Storage(format!("Couldn't upload it to Notion: {e}")))?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        if !(200..300).contains(&status) {
            let message = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("message")?.as_str().map(str::to_owned));
            return Err(Error::Invalid(message.unwrap_or_else(|| format!("Notion refused the upload ({status})."))));
        }
        Ok(id)
    }
}

#[async_trait]
impl WorkStore for Notion {
    async fn handed_in(&self, task_id: &TaskId) -> Result<Vec<HandedInFile>> {
        let task_id = task_id.clone();
        self.call(move |inner| {
            let (files, checks) = inner.page_blocks(&task_id)?;
            if let Ok(page) = inner.page(&task_id) {
                inner.remember(&task_id, page.edited, files.clone(), checks);
            }
            Ok(files)
        })
        .await
    }

    async fn all_handed_in(&self) -> Result<Vec<HandedInFile>> {
        self.call(|inner| {
            let pages = inner.pages()?;
            inner.rescan(&pages, |_| false)?;
            let ids: std::collections::HashSet<&str> = pages.iter().map(|p| p.id.as_str()).collect();
            let work = inner.work.lock().unwrap_or_else(|e| e.into_inner());
            Ok(work.iter().filter(|(id, _)| ids.contains(id.as_str())).flat_map(|(_, scan)| scan.files.clone()).collect())
        })
        .await
    }

    async fn hand_in(&self, file: HandedInFile, bytes: Vec<u8>, device_name: &str) -> Result<HandedInFile> {
        let caption = handed_in_caption(&file, device_name);
        self.call(move |inner| {
            let upload = inner.upload(&file.name, &bytes)?;
            let block = json!({
                "type": "file",
                "file": {
                    "type": "file_upload",
                    "file_upload": { "id": upload },
                    "name": file.name,
                    "caption": [{ "type": "text", "text": { "content": caption } }],
                }
            });
            let result = inner.send("PATCH", &format!("/blocks/{}/children", file.task_id), Some(&json!({ "children": [block] })))?;
            inner.forget_cache();
            inner.work.lock().unwrap_or_else(|e| e.into_inner()).remove(&file.task_id);
            result
                .get("results")
                .and_then(|r| r.as_array())
                .and_then(|r| r.iter().rev().find_map(|b| handed_in_block(&file.task_id, b)))
                .ok_or_else(|| Error::Storage("Notion changed the handed-in file so MultiTasker can't read it back.".into()))
        })
        .await
    }

    async fn remove_handed_in(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |inner| {
            inner.work.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, scan| !scan.files.iter().any(|f| f.id == id));
            match inner.send("DELETE", &format!("/blocks/{id}"), None) {
                Ok(_) | Err(Error::NotFound(_)) => Ok(()),
                Err(e) => Err(e),
            }
        })
        .await
    }

    async fn download(&self, file: &HandedInFile) -> Result<Vec<u8>> {
        let id = file.id.clone();
        self.call(move |inner| {
            // Notion's file links last an hour, so a fresh one is asked for each time.
            let block = inner.send("GET", &format!("/blocks/{id}"), None)?;
            let url = block
                .pointer("/file/file/url")
                .and_then(|u| u.as_str())
                .ok_or_else(|| Error::NotFound("That handed-in file is no longer in Notion.".into()))?;
            let mut response = inner.agent.get(url).call().map_err(|e| Error::Storage(format!("Couldn't download it from Notion: {e}")))?;
            if !response.status().is_success() {
                return Err(Error::Storage(format!("Notion answered {} for that file.", response.status().as_u16())));
            }
            response
                .body_mut()
                .with_config()
                .limit(MAX_FILE)
                .read_to_vec()
                .map_err(|e| Error::Storage(format!("Couldn't download it from Notion: {e}")))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Map<String, Value> {
        json!({
            "Name": { "type": "title", "title": {} },
            "Due": { "type": "date", "date": {} },
            "Files": { "type": "files", "files": {} },
            "Status": { "type": "status", "status": {
                "options": [
                    { "id": "a", "name": "Not started" }, { "id": "b", "name": "In progress" }, { "id": "c", "name": "Done" }
                ],
                "groups": [
                    { "name": "To-do", "option_ids": ["a"] }, { "name": "In progress", "option_ids": ["b"] },
                    { "name": "Complete", "option_ids": ["c"] }
                ]
            } }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn uses_the_databases_own_columns_and_adds_the_rest() {
        let (columns, missing) = plan_columns(&schema()).unwrap();
        assert_eq!((columns.title.as_str(), columns.due.as_str(), columns.files.as_str()), ("Name", "Due", "Files"));
        assert!(matches!(&columns.done, Done::Status { doing, done, .. } if doing == "In progress" && done == "Done"));
        let mut added: Vec<&str> = missing.keys().map(String::as_str).collect();
        added.sort();
        assert_eq!(added, ["Category", "Description", "Left off", "Minutes spent", "Started", "Started on"]);
    }

    #[test]
    fn a_database_without_a_status_gets_a_done_checkbox() {
        let props = json!({ "Task": { "type": "title", "title": {} } }).as_object().unwrap().clone();
        let (columns, missing) = plan_columns(&props).unwrap();
        assert_eq!(columns.done, Done::Checkbox("Done".into()));
        assert!(missing.contains_key("Done") && missing.contains_key("Due") && missing.contains_key("Attachments"));
    }

    #[test]
    fn local_files_round_trip_through_file_links() {
        let a = Attachment {
            id: String::new(),
            provider: PROVIDER_KEY.into(),
            task_id: "p1".into(),
            kind: AttachmentKind::Document,
            name: "GS notes (v2).docx".into(),
            target: r"C:\Users\me\School\GS notes (v2) #1 – 100%.docx".into(),
            device: Some("pc-1".into()),
            added_at: Utc::now(),
        };
        let entry = file_entry(&a);
        assert!(entry.pointer("/external/url").unwrap().as_str().unwrap().starts_with("file:///C:/Users/me/School/GS%20notes"));
        let back = file_attachment("p1", &entry, a.added_at).unwrap();
        assert_eq!((back.kind, back.target.as_str(), back.device.as_deref()), (a.kind, a.target.as_str(), Some("pc-1")));
    }

    #[test]
    fn reads_links_and_files_added_in_notion_itself() {
        let now = Utc::now();
        let link = json!({ "name": "Brief", "type": "external", "external": { "url": "https://example.com/brief" } });
        let upload = json!({ "name": "slides.pdf", "type": "file", "file": { "url": "https://s3/x?sig=1", "expiry_time": "x" } });
        let path = json!({ "name": "", "type": "external", "external": { "url": "file:///D:/Work/plan.xlsx" } });
        assert_eq!(file_attachment("p", &link, now).unwrap().kind, AttachmentKind::Link);
        assert_eq!(file_attachment("p", &upload, now).unwrap().id, "p|upload:slides.pdf");
        let path = file_attachment("p", &path, now).unwrap();
        assert_eq!((path.kind, path.name.as_str(), path.target.as_str()), (AttachmentKind::Document, "plan.xlsx", r"D:\Work\plan.xlsx"));
    }

    #[test]
    fn status_groups_decide_in_progress_and_done() {
        let (columns, _) = plan_columns(&schema()).unwrap();
        let page = |status: &str| Page {
            id: "p".into(),
            created: Utc::now(),
            edited: Utc::now(),
            properties: json!({ "Status": { "status": { "name": status } }, "Started on": { "rich_text": [{ "plain_text": "LAPTOP" }] } })
                .as_object()
                .unwrap()
                .clone(),
        };
        assert_eq!(columns.task(&page("Done")).status, TaskStatus::Done);
        let doing = columns.extras(&page("In progress"));
        assert!(doing.started_at.is_some());
        assert_eq!(doing.started_on_name.as_deref(), Some("LAPTOP"));
        assert!(columns.extras(&page("Not started")).started_at.is_none());
    }

    #[test]
    fn handed_in_files_round_trip_through_captions() {
        let file = HandedInFile {
            id: "b1".into(),
            task_id: "p1".into(),
            name: "Essay final.docx".into(),
            sha256: "cd".repeat(32),
            size: 4_321,
            device: Some("pc-1".into()),
            added_at: "2026-11-21T03:04:05.678Z".parse().unwrap(),
        };
        let block = json!({
            "id": "b1", "type": "file", "created_time": "2026-11-21T03:04:00.000Z",
            "file": { "type": "file", "name": "Essay final.docx", "file": { "url": "https://x" },
                      "caption": [{ "plain_text": handed_in_caption(&file, "MY LAPTOP") }] }
        });
        assert_eq!(handed_in_block("p1", &block), Some(file));
        let theirs = json!({ "id": "b2", "type": "file", "file": { "name": "x.pdf", "caption": [{ "plain_text": "From class" }] } });
        assert_eq!(handed_in_block("p1", &theirs), None, "files the person added themselves aren't hand-ins");
    }

    #[test]
    fn edits_keep_the_day_when_only_the_time_goes() {
        let (columns, _) = plan_columns(&schema()).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 11, 26);
        let values = columns.values(&TaskChanges { due_at: Some(None), ..Default::default() }, day).unwrap();
        assert_eq!(values["Due"], json!({ "date": { "start": "2026-11-26" } }));
        let long = "x".repeat(4500);
        let values = columns.values(&TaskChanges { description: Some(Some(long)), ..Default::default() }, None).unwrap();
        assert_eq!(values["Description"]["rich_text"].as_array().unwrap().len(), 3, "split into Notion-sized pieces");
    }

    /// Runs against a real database: `NOTION_TOKEN=... NOTION_DATABASE=<data source id> cargo test -p multitasker-provider-notion -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn works_against_a_real_database() {
        let (Ok(token), Ok(source)) = (std::env::var("NOTION_TOKEN"), std::env::var("NOTION_DATABASE")) else { return };
        let (_, databases) = Notion::databases(&token).await.unwrap();
        assert!(databases.iter().any(|d| d.id == source));

        let notion = Notion::new(token, source);
        notion.prepare().await.unwrap();
        let due = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 12, 1, 8, 0, 0).unwrap();
        let task = notion
            .create(NewTask { title: "MultiTasker live test".into(), category: Some("School".into()), due_at: Some(due), ..Default::default() })
            .await
            .unwrap();
        assert_eq!((task.category.as_deref(), task.due_at), (Some("School"), Some(due)));

        notion.start(&task.id, &"pc-test".to_owned(), "TEST-PC").await.unwrap();
        let file = Attachment {
            id: String::new(),
            provider: PROVIDER_KEY.into(),
            task_id: task.id.clone(),
            kind: AttachmentKind::Pdf,
            name: "Brief.pdf".into(),
            target: r"C:\Users\me\Brief v2.pdf".into(),
            device: Some("pc-test".into()),
            added_at: Utc::now(),
        };
        let stored = notion.add_attachment(file).await.unwrap();
        let extras = ExtrasStore::extras(&notion).await.unwrap().remove(&task.id).unwrap();
        assert_eq!(extras.started_on_name.as_deref(), Some("TEST-PC"));
        assert_eq!(extras.attachments.iter().map(|a| a.target.as_str()).collect::<Vec<_>>(), [r"C:\Users\me\Brief v2.pdf"]);

        let edited = notion.update(&task.id, TaskChanges { due_at: Some(None), ..Default::default() }).await.unwrap();
        assert_eq!((edited.due_at, edited.due_date), (None, Some(due.with_timezone(&Local).date_naive())));
        let done = notion.update(&task.id, TaskChanges { status: Some(TaskStatus::Done), ..Default::default() }).await.unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        let extras = ExtrasStore::extras(&notion).await.unwrap().remove(&task.id).unwrap_or_default();
        assert!(extras.started_at.is_none(), "finishing it stopped it");

        notion.remove_attachment(&stored.id).await.unwrap();

        // Time adds up across pauses, and the newest left-off note replaces the last.
        notion.add_worked(&task.id, 90).await.unwrap();
        notion.add_worked(&task.id, 30).await.unwrap();
        let note = |text: &str| LeftOff { text: text.into(), at: Utc::now(), device: "pc-test".into() };
        notion.set_left_off(&task.id, Some(note("Intro done"))).await.unwrap();
        notion.set_left_off(&task.id, Some(note("Halfway through part 2"))).await.unwrap();
        let extras = ExtrasStore::extras(&notion).await.unwrap().remove(&task.id).unwrap();
        assert_eq!(extras.worked_secs, 120);
        assert_eq!(extras.left_off.map(|n| n.text), Some("Halfway through part 2".into()));
        notion.set_left_off(&task.id, None).await.unwrap();

        // Checklist steps live in the list's own sub-tasks or to-dos, ticked off and removed there too.
        let outline = notion.add_check(&task.id, "Outline").await.unwrap();
        notion.add_check(&task.id, "Draft").await.unwrap();
        notion.set_check(&task.id, &outline.id, true).await.unwrap();
        let steps = ExtrasStore::extras(&notion).await.unwrap().remove(&task.id).unwrap().checklist;
        let shown: Vec<(String, bool)> = steps.iter().map(|c| (c.text.clone(), c.done)).collect();
        assert_eq!(shown, [("Outline".to_owned(), true), ("Draft".to_owned(), false)]);
        assert!(notion.list().await.unwrap().iter().all(|t| t.title != "Outline"), "steps aren't tasks of their own");
        notion.remove_check(&task.id, &outline.id).await.unwrap();
        let left = ExtrasStore::extras(&notion).await.unwrap().remove(&task.id).unwrap().checklist;
        assert_eq!(left.len(), 1);
        let cleared = ExtrasStore::extras(&notion).await.unwrap().remove(&task.id).and_then(|e| e.left_off);
        assert_eq!(cleared, None);

        notion.delete(&task.id).await.unwrap();
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
    async fn hands_in_work_to_a_real_database() {
        let (Ok(token), Ok(source)) = (std::env::var("NOTION_TOKEN"), std::env::var("NOTION_DATABASE")) else { return };
        let notion = Notion::new(token, source);
        let task = notion.create(NewTask { title: "MultiTasker hand-in test".into(), ..Default::default() }).await.unwrap();
        hand_in_round_trip(&notion, &task).await;
        notion.delete(&task.id).await.unwrap();
    }
}
