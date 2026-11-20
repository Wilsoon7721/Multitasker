use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate, SecondsFormat, Utc};
use multitasker_core::{
    check_category, check_title, clean, Attachment, AttachmentKind, DeviceId, Error, ExtrasStore, NewTask, Result, Task,
    TaskChanges, TaskExtras, TaskId, TaskProvider, TaskStatus,
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

fn new_agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into()
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
            pages.extend(result.get("results").and_then(|v| v.as_array()).into_iter().flatten().filter_map(page_of));
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
    Ok((Columns { title, done, due, category: Some(category), description, started, started_on, files }, missing))
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
        if self.group(page) != Group::Doing {
            return TaskExtras { attachments, ..Default::default() };
        }
        let name = plain(page.properties.get(&self.started_on).and_then(|p| p.get("rich_text")));
        TaskExtras {
            // Moved to In progress in Notion itself, it has no start time of its own yet.
            started_at: Some(self.started_at(page).unwrap_or(page.edited)),
            started_on: None,
            started_on_name: clean(Some(name)),
            attachments,
        }
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
            Ok(inner
                .pages()?
                .iter()
                .map(|p| (p.id.clone(), columns.extras(p)))
                .filter(|(_, e)| e.started_at.is_some() || !e.attachments.is_empty())
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
        assert_eq!(added, ["Category", "Description", "Started", "Started on"]);
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
        notion.delete(&task.id).await.unwrap();
    }
}
