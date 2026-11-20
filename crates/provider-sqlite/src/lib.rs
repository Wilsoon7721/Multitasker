use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use multitasker_core::{
    check_category, check_title, clean, Attachment, AttachmentKind, Error, FocusEntry, NewTask, Result, StateStore, Task,
    TaskChanges, TaskId, TaskProvider, TaskStatus,
};
use rusqlite::{params, Connection, OptionalExtension, Row};

pub const PROVIDER_KEY: &str = "sqlite";

/// Each entry migrates the schema one version forward; the index + 1 is the version it leaves the file at.
const MIGRATIONS: &[&str] = &[r#"
    CREATE TABLE tasks (
        id            TEXT PRIMARY KEY,
        title         TEXT NOT NULL,
        description   TEXT,
        category      TEXT,
        due_date      TEXT,
        status        TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'done')),
        created_at    TEXT NOT NULL,
        updated_at    TEXT NOT NULL,
        completed_at  TEXT
    );

    CREATE TABLE focus (
        provider    TEXT NOT NULL,
        task_id     TEXT NOT NULL,
        device      TEXT NOT NULL,
        started_at  TEXT NOT NULL,
        PRIMARY KEY (provider, task_id, device)
    );
"#, r#"
    CREATE TABLE attachments (
        id        TEXT PRIMARY KEY,
        provider  TEXT NOT NULL,
        task_id   TEXT NOT NULL,
        kind      TEXT NOT NULL,
        name      TEXT NOT NULL,
        target    TEXT NOT NULL,
        device    TEXT,
        added_at  TEXT NOT NULL,
        UNIQUE (provider, task_id, target)
    );
"#];

/// Tasks and MultiTasker's own state in one local file, on separate tables so either can be paired with others.
pub struct SqliteDb {
    conn: Mutex<Connection>,
}

impl SqliteDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path).map_err(storage)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(storage)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(storage)?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(storage)?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(sql).map_err(storage)?;
            conn.pragma_update(None, "user_version", (i + 1) as i64).map_err(storage)?;
        }

        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic mid-query can't leave SQLite itself inconsistent, so a poisoned lock is safe to reuse.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn get(conn: &Connection, id: &str) -> Result<Task> {
        conn.query_row("SELECT * FROM tasks WHERE id = ?1", [id], task_from_row)
            .optional()
            .map_err(storage)?
            .ok_or_else(|| Error::NotFound(id.to_owned()))
    }
}

#[async_trait]
impl TaskProvider for SqliteDb {
    fn key(&self) -> &str {
        PROVIDER_KEY
    }

    fn display_name(&self) -> &str {
        "This computer"
    }

    async fn list(&self) -> Result<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT * FROM tasks ORDER BY status = 'done', due_date IS NULL, due_date, created_at")
            .map_err(storage)?;
        let tasks = stmt.query_map([], task_from_row).map_err(storage)?;
        tasks.collect::<rusqlite::Result<_>>().map_err(storage)
    }

    async fn create(&self, new: NewTask) -> Result<Task> {
        let now = Utc::now();
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            title: check_title(&new.title)?,
            description: clean(new.description),
            category: check_category(new.category)?,
            due_date: new.due_date,
            status: TaskStatus::Todo,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        self.conn()
            .execute(
                "INSERT INTO tasks (id, title, description, category, due_date, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    task.id,
                    task.title,
                    task.description,
                    task.category,
                    task.due_date,
                    task.status.as_str(),
                    task.created_at,
                    task.updated_at
                ],
            )
            .map_err(storage)?;

        Ok(task)
    }

    async fn update(&self, id: &TaskId, changes: TaskChanges) -> Result<Task> {
        let conn = self.conn();
        let mut task = Self::get(&conn, id)?;

        if let Some(title) = changes.title {
            task.title = check_title(&title)?;
        }
        if let Some(description) = changes.description {
            task.description = clean(description);
        }
        if let Some(category) = changes.category {
            task.category = check_category(category)?;
        }
        if let Some(due_date) = changes.due_date {
            task.due_date = due_date;
        }
        if let Some(status) = changes.status {
            if status != task.status {
                task.completed_at = (status == TaskStatus::Done).then(Utc::now);
            }
            task.status = status;
        }
        task.updated_at = Utc::now();

        conn.execute(
            "UPDATE tasks SET title = ?2, description = ?3, category = ?4, due_date = ?5, status = ?6,
                              updated_at = ?7, completed_at = ?8
             WHERE id = ?1",
            params![
                task.id,
                task.title,
                task.description,
                task.category,
                task.due_date,
                task.status.as_str(),
                task.updated_at,
                task.completed_at
            ],
        )
        .map_err(storage)?;

        Ok(task)
    }

    async fn delete(&self, id: &TaskId) -> Result<()> {
        let removed = self.conn().execute("DELETE FROM tasks WHERE id = ?1", [id]).map_err(storage)?;
        if removed == 0 {
            return Err(Error::NotFound(id.clone()));
        }
        Ok(())
    }
}

#[async_trait]
impl StateStore for SqliteDb {
    async fn focus(&self) -> Result<Vec<FocusEntry>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT provider, task_id, device, started_at FROM focus ORDER BY started_at")
            .map_err(storage)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FocusEntry { provider: r.get(0)?, task_id: r.get(1)?, device: r.get(2)?, started_at: r.get(3)? })
            })
            .map_err(storage)?;
        rows.collect::<rusqlite::Result<_>>().map_err(storage)
    }

    async fn start(&self, provider: &str, task_id: &TaskId, device: &String) -> Result<FocusEntry> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO focus (provider, task_id, device, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![provider, task_id, device, Utc::now()],
        )
        .map_err(storage)?;

        let started_at: DateTime<Utc> = conn
            .query_row(
                "SELECT started_at FROM focus WHERE provider = ?1 AND task_id = ?2 AND device = ?3",
                params![provider, task_id, device],
                |r| r.get(0),
            )
            .map_err(storage)?;

        Ok(FocusEntry { provider: provider.to_owned(), task_id: task_id.clone(), device: device.clone(), started_at })
    }

    async fn stop(&self, provider: &str, task_id: &TaskId) -> Result<()> {
        self.conn()
            .execute("DELETE FROM focus WHERE provider = ?1 AND task_id = ?2", params![provider, task_id])
            .map_err(storage)?;
        Ok(())
    }

    async fn attachments(&self) -> Result<Vec<Attachment>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM attachments ORDER BY added_at").map_err(storage)?;
        let rows = stmt.query_map([], attachment_from_row).map_err(storage)?;
        rows.collect::<rusqlite::Result<_>>().map_err(storage)
    }

    async fn add_attachment(&self, a: Attachment) -> Result<Attachment> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO attachments (id, provider, task_id, kind, name, target, device, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![a.id, a.provider, a.task_id, a.kind.as_str(), a.name, a.target, a.device, a.added_at],
        )
        .map_err(storage)?;

        // Either the row just added, or the one already there for the same target.
        conn.query_row(
            "SELECT * FROM attachments WHERE provider = ?1 AND task_id = ?2 AND target = ?3",
            params![a.provider, a.task_id, a.target],
            attachment_from_row,
        )
        .map_err(storage)
    }

    async fn remove_attachment(&self, id: &str) -> Result<()> {
        let removed = self.conn().execute("DELETE FROM attachments WHERE id = ?1", [id]).map_err(storage)?;
        if removed == 0 {
            return Err(Error::NotFound(id.to_owned()));
        }
        Ok(())
    }

    async fn forget(&self, provider: &str, task_id: &TaskId) -> Result<()> {
        let conn = self.conn();
        for table in ["focus", "attachments"] {
            conn.execute(&format!("DELETE FROM {table} WHERE provider = ?1 AND task_id = ?2"), params![provider, task_id])
                .map_err(storage)?;
        }
        Ok(())
    }
}

fn attachment_from_row(row: &Row) -> rusqlite::Result<Attachment> {
    let kind: String = row.get("kind")?;
    Ok(Attachment {
        id: row.get("id")?,
        provider: row.get("provider")?,
        task_id: row.get("task_id")?,
        kind: AttachmentKind::parse(&kind).map_err(conversion)?,
        name: row.get("name")?,
        target: row.get("target")?,
        device: row.get("device")?,
        added_at: row.get("added_at")?,
    })
}

fn conversion(e: Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

fn task_from_row(row: &Row) -> rusqlite::Result<Task> {
    let status: String = row.get("status")?;
    Ok(Task {
        id: row.get("id")?,
        title: row.get("title")?,
        description: row.get("description")?,
        category: row.get("category")?,
        due_date: row.get::<_, Option<NaiveDate>>("due_date")?,
        status: TaskStatus::parse(&status).map_err(conversion)?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        completed_at: row.get("completed_at")?,
    })
}

fn storage(e: rusqlite::Error) -> Error {
    Error::Storage(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(title: &str) -> NewTask {
        NewTask { title: title.into(), ..Default::default() }
    }

    #[tokio::test]
    async fn creates_lists_updates_and_deletes() {
        let db = SqliteDb::open_in_memory().unwrap();

        let task = db.create(NewTask { category: Some(" school ".into()), ..new(" GS essay ") }).await.unwrap();
        assert_eq!(task.title, "GS essay");
        assert_eq!(task.category.as_deref(), Some("school"));

        let done = db
            .update(&task.id, TaskChanges { status: Some(TaskStatus::Done), category: Some(None), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert!(done.completed_at.is_some());
        assert_eq!(done.category, None);

        assert_eq!(db.list().await.unwrap(), vec![done.clone()]);

        db.delete(&task.id).await.unwrap();
        assert!(db.list().await.unwrap().is_empty());
        assert!(matches!(db.delete(&task.id).await, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn rejects_blank_titles() {
        let db = SqliteDb::open_in_memory().unwrap();
        assert!(matches!(db.create(new("  ")).await, Err(Error::Invalid(_))));
    }

    #[tokio::test]
    async fn focus_keeps_first_start_and_stops_everywhere() {
        let db = SqliteDb::open_in_memory().unwrap();
        let id = "t1".to_string();

        let first = db.start(PROVIDER_KEY, &id, &"pc-1".into()).await.unwrap();
        let again = db.start(PROVIDER_KEY, &id, &"pc-1".into()).await.unwrap();
        assert_eq!(first.started_at, again.started_at);

        db.start(PROVIDER_KEY, &id, &"pc-2".into()).await.unwrap();
        assert_eq!(db.focus().await.unwrap().len(), 2);

        db.stop(PROVIDER_KEY, &id).await.unwrap();
        assert!(db.focus().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn attachments_dedupe_by_target_and_are_forgotten_with_the_task() {
        use multitasker_core::NewAttachment;

        let db = SqliteDb::open_in_memory().unwrap();
        let (task, device) = ("t1".to_string(), "pc-1".to_string());
        let link = |url: &str| {
            Attachment::new(PROVIDER_KEY, &task, &device, NewAttachment::Link { url: url.into(), name: None }).unwrap()
        };

        let first = db.add_attachment(link("https://claude.ai/chat/1")).await.unwrap();
        let again = db.add_attachment(link("https://claude.ai/chat/1")).await.unwrap();
        assert_eq!(first.id, again.id);

        let other = db.add_attachment(link("example.com")).await.unwrap();
        assert_eq!(other.kind, AttachmentKind::Link);
        assert_eq!(db.attachments().await.unwrap().len(), 2);

        db.remove_attachment(&other.id).await.unwrap();
        assert!(matches!(db.remove_attachment(&other.id).await, Err(Error::NotFound(_))));

        db.start(PROVIDER_KEY, &task, &device).await.unwrap();
        db.forget(PROVIDER_KEY, &task).await.unwrap();
        assert!(db.attachments().await.unwrap().is_empty());
        assert!(db.focus().await.unwrap().is_empty());
    }

    #[test]
    fn reopening_a_file_keeps_data_and_skips_migrations() {
        let dir = std::env::temp_dir().join(format!("multitasker-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tasks.db");

        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            SqliteDb::open(&path).unwrap().create(new("persisted")).await.unwrap();
            let tasks = SqliteDb::open(&path).unwrap().list().await.unwrap();
            assert_eq!(tasks.len(), 1);
        });

        let _ = std::fs::remove_dir_all(dir);
    }
}
