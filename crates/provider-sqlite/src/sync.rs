use chrono::{DateTime, Utc};
use multitasker_core::{Change, DeviceInfo, Op, Result, Stamp, Task, TaskFields, TaskStatus};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{attachment_from_row, insert_attachment, storage, task_from_row, SqliteDb, PROVIDER_KEY};

/// The device id sync runs as; no row means sync is off and nothing is logged.
const DEVICE: &str = "device";
/// The newest stamp this database has made or seen.
const CLOCK: &str = "clock";
/// The number of this device's last uploaded batch.
const SEGMENT: &str = "segment";

fn meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |r| r.get(0)).optional().map_err(storage)
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO sync_meta (key, value) VALUES (?1, ?2)", [key, value]).map_err(storage)?;
    Ok(())
}

pub(crate) fn all_fields(task: &Task) -> TaskFields {
    TaskFields {
        title: Some(task.title.clone()),
        description: Some(task.description.clone()),
        category: Some(task.category.clone()),
        due_date: Some(task.due_date),
        due_at: Some(task.due_at),
        status: Some(task.status),
        completed_at: Some(task.completed_at),
        created_at: Some(task.created_at),
    }
}

pub(crate) fn changed_fields(before: &Task, after: &Task) -> TaskFields {
    TaskFields {
        title: (before.title != after.title).then(|| after.title.clone()),
        description: (before.description != after.description).then(|| after.description.clone()),
        category: (before.category != after.category).then(|| after.category.clone()),
        due_date: (before.due_date != after.due_date).then_some(after.due_date),
        due_at: (before.due_at != after.due_at).then_some(after.due_at),
        status: (before.status != after.status).then_some(after.status),
        completed_at: (before.completed_at != after.completed_at).then_some(after.completed_at),
        created_at: None,
    }
}

/// Logs a local change for other devices when sync is on.
pub(crate) fn record(conn: &Connection, change: Change) -> Result<()> {
    let Some(device) = meta(conn, DEVICE)? else { return Ok(()) };
    let last = meta(conn, CLOCK)?.and_then(|s| Stamp::parse(&s));
    let stamp = Stamp::next(last.as_ref(), Utc::now().timestamp_millis(), &device);
    set_meta(conn, CLOCK, stamp.as_str())?;

    // Stamp it locally too, so an older change arriving later can't undo it.
    mark(conn, &change, &stamp)?;
    let op = serde_json::to_string(&Op { stamp, change }).map_err(|e| multitasker_core::Error::Storage(e.to_string()))?;
    conn.execute("INSERT INTO sync_outbox (op) VALUES (?1)", [op]).map_err(storage)?;
    Ok(())
}

/// Clears a task's in-progress mark on every device, logging one change per device.
pub(crate) fn stop_focus(conn: &Connection, provider: &str, task_id: &str) -> Result<()> {
    let devices: Vec<String> = {
        let mut stmt = conn.prepare("SELECT device FROM focus WHERE provider = ?1 AND task_id = ?2").map_err(storage)?;
        let rows = stmt.query_map(params![provider, task_id], |r| r.get(0)).map_err(storage)?;
        rows.collect::<rusqlite::Result<_>>().map_err(storage)?
    };
    conn.execute("DELETE FROM focus WHERE provider = ?1 AND task_id = ?2", params![provider, task_id]).map_err(storage)?;
    for device in devices {
        record(conn, Change::Focus { provider: provider.into(), task_id: task_id.into(), device, started_at: None })?;
    }
    Ok(())
}

fn focus_key(provider: &str, task_id: &str, device: &str) -> String {
    format!("{provider}\u{1f}{task_id}\u{1f}{device}")
}

fn field_names(fields: &TaskFields) -> Vec<&'static str> {
    let mut names = Vec::new();
    let mut add = |present: bool, name| present.then(|| names.push(name));
    add(fields.title.is_some(), "title");
    add(fields.description.is_some(), "description");
    add(fields.category.is_some(), "category");
    add(fields.due_date.is_some(), "due_date");
    add(fields.due_at.is_some(), "due_at");
    add(fields.status.is_some(), "status");
    add(fields.completed_at.is_some(), "completed_at");
    add(fields.created_at.is_some(), "created_at");
    names
}

fn clock(conn: &Connection, entity: &str, key: &str, field: &str) -> Result<Option<Stamp>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT stamp FROM sync_clocks WHERE entity = ?1 AND key = ?2 AND field = ?3",
            params![entity, key, field],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    Ok(raw.and_then(|s| Stamp::parse(&s)))
}

fn set_clock(conn: &Connection, entity: &str, key: &str, field: &str, stamp: &Stamp) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sync_clocks (entity, key, field, stamp) VALUES (?1, ?2, ?3, ?4)",
        params![entity, key, field, stamp.as_str()],
    )
    .map_err(storage)?;
    Ok(())
}

fn is_tombstoned(conn: &Connection, entity: &str, key: &str) -> Result<bool> {
    conn.query_row("SELECT 1 FROM sync_tombstones WHERE entity = ?1 AND key = ?2", params![entity, key], |_| Ok(()))
        .optional()
        .map(|found| found.is_some())
        .map_err(storage)
}

fn set_tombstone(conn: &Connection, entity: &str, key: &str, stamp: &Stamp) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sync_tombstones (entity, key, stamp) VALUES (?1, ?2, ?3)",
        params![entity, key, stamp.as_str()],
    )
    .map_err(storage)?;
    Ok(())
}

fn mark(conn: &Connection, change: &Change, stamp: &Stamp) -> Result<()> {
    match change {
        Change::Task { id, fields } => {
            for name in field_names(fields) {
                set_clock(conn, "task", id, name, stamp)?;
            }
        }
        Change::TaskDeleted { id } => set_tombstone(conn, "task", id, stamp)?,
        Change::Focus { provider, task_id, device, .. } => {
            set_clock(conn, "focus", &focus_key(provider, task_id, device), "", stamp)?
        }
        Change::AttachmentAdded { .. } => {}
        Change::AttachmentRemoved { id } => set_tombstone(conn, "attachment", id, stamp)?,
    }
    Ok(())
}

/// Moves this database's clock past a stamp from another device, so its next change sorts after it.
fn observe(conn: &Connection, stamp: &Stamp) -> Result<()> {
    let last = meta(conn, CLOCK)?.and_then(|s| Stamp::parse(&s));
    if last.as_ref().is_none_or(|l| l < stamp) {
        set_meta(conn, CLOCK, stamp.as_str())?;
    }
    Ok(())
}

/// Applies one remote change, newest-wins per field; returns whether anything visible changed.
fn apply(conn: &Connection, op: &Op) -> Result<bool> {
    observe(conn, &op.stamp)?;
    let stamp = &op.stamp;
    match &op.change {
        Change::Task { id, fields } => {
            if is_tombstoned(conn, "task", id)? {
                return Ok(false);
            }
            let now = Utc::now();
            // An edit can arrive before the task's creation does; the placeholder is filled in by later changes.
            conn.execute(
                "INSERT OR IGNORE INTO tasks (id, title, status, created_at, updated_at) VALUES (?1, '', 'todo', ?2, ?3)",
                params![id, fields.created_at.unwrap_or(now), now],
            )
            .map_err(storage)?;

            let mut changed = false;
            for name in field_names(fields) {
                if clock(conn, "task", id, name)?.is_some_and(|c| &c >= stamp) {
                    continue;
                }
                set_field(conn, id, name, fields)?;
                set_clock(conn, "task", id, name, stamp)?;
                changed = true;
            }
            if changed {
                conn.execute("UPDATE tasks SET updated_at = ?2 WHERE id = ?1", params![id, now]).map_err(storage)?;
            }
            Ok(changed)
        }
        Change::TaskDeleted { id } => {
            set_tombstone(conn, "task", id, stamp)?;
            let removed = conn.execute("DELETE FROM tasks WHERE id = ?1", [id]).map_err(storage)?;
            for table in ["focus", "attachments"] {
                conn.execute(&format!("DELETE FROM {table} WHERE provider = ?1 AND task_id = ?2"), params![PROVIDER_KEY, id])
                    .map_err(storage)?;
            }
            Ok(removed > 0)
        }
        Change::Focus { provider, task_id, device, started_at } => {
            let key = focus_key(provider, task_id, device);
            if clock(conn, "focus", &key, "")?.is_some_and(|c| &c >= stamp) {
                return Ok(false);
            }
            set_clock(conn, "focus", &key, "", stamp)?;
            match started_at {
                Some(at) => conn.execute(
                    "INSERT OR REPLACE INTO focus (provider, task_id, device, started_at) VALUES (?1, ?2, ?3, ?4)",
                    params![provider, task_id, device, at],
                ),
                None => conn.execute(
                    "DELETE FROM focus WHERE provider = ?1 AND task_id = ?2 AND device = ?3",
                    params![provider, task_id, device],
                ),
            }
            .map_err(storage)?;
            Ok(true)
        }
        Change::AttachmentAdded { attachment } => {
            if is_tombstoned(conn, "attachment", &attachment.id)? {
                return Ok(false);
            }
            insert_attachment(conn, attachment)
        }
        Change::AttachmentRemoved { id } => {
            set_tombstone(conn, "attachment", id, stamp)?;
            Ok(conn.execute("DELETE FROM attachments WHERE id = ?1", [id]).map_err(storage)? > 0)
        }
    }
}

fn set_field(conn: &Connection, id: &str, name: &str, f: &TaskFields) -> Result<()> {
    let set = |column: &str, value: &dyn rusqlite::ToSql| {
        conn.execute(&format!("UPDATE tasks SET {column} = ?2 WHERE id = ?1"), params![id, value]).map_err(storage)
    };
    match name {
        "title" => set("title", &f.title),
        "description" => set("description", &f.description.clone().flatten()),
        "category" => set("category", &f.category.clone().flatten()),
        "due_date" => set("due_date", &f.due_date.flatten()),
        "due_at" => set("due_at", &f.due_at.flatten()),
        "status" => set("status", &f.status.unwrap_or(TaskStatus::Todo).as_str()),
        "completed_at" => set("completed_at", &f.completed_at.flatten()),
        _ => set("created_at", &f.created_at),
    }?;
    Ok(())
}

impl SqliteDb {
    /// The device id sync runs as, or `None` while sync is off.
    pub fn sync_device(&self) -> Result<Option<String>> {
        meta(&self.conn(), DEVICE)
    }

    /// Turns on change logging for `device`. The first time, everything already here is queued so it gets uploaded.
    pub fn enable_sync(&self, device: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(storage)?;
        if meta(&tx, DEVICE)?.as_deref() == Some(device) {
            return Ok(());
        }
        set_meta(&tx, DEVICE, device)?;

        let tasks: Vec<Task> = {
            let mut stmt = tx.prepare("SELECT * FROM tasks").map_err(storage)?;
            let rows = stmt.query_map([], task_from_row).map_err(storage)?;
            rows.collect::<rusqlite::Result<_>>().map_err(storage)?
        };
        for task in &tasks {
            record(&tx, Change::Task { id: task.id.clone(), fields: all_fields(task) })?;
        }

        let focus: Vec<(String, String, String, DateTime<Utc>)> = {
            let mut stmt = tx.prepare("SELECT provider, task_id, device, started_at FROM focus").map_err(storage)?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(storage)?;
            rows.collect::<rusqlite::Result<_>>().map_err(storage)?
        };
        for (provider, task_id, device, at) in focus {
            record(&tx, Change::Focus { provider, task_id, device, started_at: Some(at) })?;
        }

        let attachments: Vec<_> = {
            let mut stmt = tx.prepare("SELECT * FROM attachments").map_err(storage)?;
            let rows = stmt.query_map([], attachment_from_row).map_err(storage)?;
            rows.collect::<rusqlite::Result<_>>().map_err(storage)?
        };
        for attachment in attachments {
            record(&tx, Change::AttachmentAdded { attachment })?;
        }
        tx.commit().map_err(storage)
    }

    /// Stops logging changes and drops anything not yet uploaded; turning sync back on re-queues everything.
    pub fn disable_sync(&self) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM sync_meta WHERE key = ?1", [DEVICE]).map_err(storage)?;
        conn.execute("DELETE FROM sync_outbox", []).map_err(storage)?;
        Ok(())
    }

    /// Forgets how far sync got, so a new location gets everything uploaded and every batch already there read.
    pub fn reset_sync(&self) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(storage)?;
        tx.execute("DELETE FROM sync_meta WHERE key IN (?1, ?2)", [DEVICE, SEGMENT]).map_err(storage)?;
        tx.execute("DELETE FROM sync_outbox", []).map_err(storage)?;
        tx.execute("DELETE FROM sync_cursors", []).map_err(storage)?;
        tx.commit().map_err(storage)
    }

    /// Changes waiting to be uploaded, oldest first, with their queue positions.
    pub fn pending_ops(&self, limit: usize) -> Result<Vec<(i64, Op)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT seq, op FROM sync_outbox ORDER BY seq LIMIT ?1").map_err(storage)?;
        let rows = stmt
            .query_map([limit as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?;
        rows.into_iter()
            .map(|(seq, json)| {
                serde_json::from_str(&json).map(|op| (seq, op)).map_err(|e| multitasker_core::Error::Storage(e.to_string()))
            })
            .collect()
    }

    /// Marks queued changes up to `through` as uploaded in batch number `segment`.
    pub fn ack_ops(&self, through: i64, segment: u64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(storage)?;
        tx.execute("DELETE FROM sync_outbox WHERE seq <= ?1", [through]).map_err(storage)?;
        set_meta(&tx, SEGMENT, &segment.to_string())?;
        tx.commit().map_err(storage)
    }

    /// The number of this device's last uploaded batch (0 before the first).
    pub fn last_segment(&self) -> Result<u64> {
        Ok(meta(&self.conn(), SEGMENT)?.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// The last batch already merged from another device.
    pub fn cursor(&self, device: &str) -> Result<u64> {
        let conn = self.conn();
        let segment: Option<i64> = conn
            .query_row("SELECT segment FROM sync_cursors WHERE device = ?1", [device], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        Ok(segment.unwrap_or(0) as u64)
    }

    /// Merges one batch from another device and moves its cursor, together; returns how many changes took effect.
    pub fn apply_ops(&self, device: &str, segment: u64, ops: &[Op]) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(storage)?;
        let mut applied = 0;
        for op in ops {
            if apply(&tx, op)? {
                applied += 1;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO sync_cursors (device, segment) VALUES (?1, ?2)",
            params![device, segment as i64],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(applied)
    }

    pub fn save_devices(&self, devices: &[DeviceInfo]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(storage)?;
        for d in devices {
            tx.execute(
                "INSERT OR REPLACE INTO devices (id, name, last_seen) VALUES (?1, ?2, ?3)",
                params![d.id, d.name, d.last_seen],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, last_seen FROM devices ORDER BY name").map_err(storage)?;
        let rows = stmt
            .query_map([], |r| Ok(DeviceInfo { id: r.get(0)?, name: r.get(1)?, last_seen: r.get(2)? }))
            .map_err(storage)?;
        rows.collect::<rusqlite::Result<_>>().map_err(storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitasker_core::{Attachment, NewAttachment, NewTask, StateStore, TaskChanges, TaskProvider};

    async fn pair() -> (SqliteDb, SqliteDb) {
        let (a, b) = (SqliteDb::open_in_memory().unwrap(), SqliteDb::open_in_memory().unwrap());
        a.enable_sync("pc-a").unwrap();
        b.enable_sync("pc-b").unwrap();
        (a, b)
    }

    /// Ships everything `from` has queued to `to`, the way an upload and a download would.
    fn ship(from: &SqliteDb, from_device: &str, to: &SqliteDb) -> usize {
        let ops: Vec<Op> = from.pending_ops(10_000).unwrap().into_iter().map(|(_, op)| op).collect();
        let segment = to.cursor(from_device).unwrap() + 1;
        to.apply_ops(from_device, segment, &ops).unwrap()
    }

    fn drain(db: &SqliteDb) {
        if let Some((seq, _)) = db.pending_ops(10_000).unwrap().last() {
            db.ack_ops(*seq, db.last_segment().unwrap() + 1).unwrap();
        }
    }

    fn new(title: &str) -> NewTask {
        NewTask { title: title.into(), ..Default::default() }
    }

    #[tokio::test]
    async fn tasks_focus_and_attachments_reach_the_other_device() {
        let (a, b) = pair().await;
        let task = a.create(new("GS essay")).await.unwrap();
        a.start(PROVIDER_KEY, &task.id, &"pc-a".into()).await.unwrap();
        let link = Attachment::new(PROVIDER_KEY, &task.id, &"pc-a".into(), NewAttachment::Link { url: "claude.ai/chat/1".into(), name: None })
            .unwrap();
        a.add_attachment(link).await.unwrap();

        assert_eq!(ship(&a, "pc-a", &b), 3);
        assert_eq!(b.list().await.unwrap()[0].title, "GS essay");
        assert_eq!(b.focus().await.unwrap()[0].device, "pc-a");
        assert_eq!(b.attachments().await.unwrap()[0].name, "claude.ai");

        // Nothing received is re-queued, so changes don't bounce back and forth.
        assert!(b.pending_ops(10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn edits_to_different_fields_both_survive() {
        let (a, b) = pair().await;
        let task = a.create(new("Draft")).await.unwrap();
        ship(&a, "pc-a", &b);
        drain(&a);

        a.update(&task.id, TaskChanges { title: Some("Final draft".into()), ..Default::default() }).await.unwrap();
        b.update(&task.id, TaskChanges { status: Some(TaskStatus::Done), ..Default::default() }).await.unwrap();
        ship(&a, "pc-a", &b);
        ship(&b, "pc-b", &a);

        for db in [&a, &b] {
            let t = &db.list().await.unwrap()[0];
            assert_eq!((t.title.as_str(), t.status), ("Final draft", TaskStatus::Done));
            assert!(t.completed_at.is_some());
        }
    }

    #[tokio::test]
    async fn the_newest_edit_to_a_field_wins_everywhere() {
        let (a, b) = pair().await;
        let task = a.create(new("Draft")).await.unwrap();
        ship(&a, "pc-a", &b);
        drain(&a);

        a.update(&task.id, TaskChanges { title: Some("From A".into()), ..Default::default() }).await.unwrap();
        // B has seen A's edit before making its own, so B's stamp is newer.
        ship(&a, "pc-a", &b);
        b.update(&task.id, TaskChanges { title: Some("From B".into()), ..Default::default() }).await.unwrap();
        ship(&b, "pc-b", &a);
        ship(&a, "pc-a", &b);

        assert_eq!(a.list().await.unwrap()[0].title, "From B");
        assert_eq!(b.list().await.unwrap()[0].title, "From B");
    }

    #[tokio::test]
    async fn due_times_reach_the_other_device() {
        let (a, b) = pair().await;
        let task = a.create(new("Timed")).await.unwrap();
        ship(&a, "pc-a", &b);
        drain(&a);

        let at = Utc::now() + chrono::Duration::hours(3);
        a.update(&task.id, TaskChanges { due_at: Some(Some(at)), ..Default::default() }).await.unwrap();
        ship(&a, "pc-a", &b);
        let t = &b.list().await.unwrap()[0];
        assert_eq!((t.due_at.map(|d| d.timestamp()), t.due_date), (Some(at.timestamp()), Some(at.date_naive())));
    }

    #[tokio::test]
    async fn deletes_win_over_edits() {
        let (a, b) = pair().await;
        let task = a.create(new("Old")).await.unwrap();
        ship(&a, "pc-a", &b);
        drain(&a);

        a.delete(&task.id).await.unwrap();
        b.update(&task.id, TaskChanges { title: Some("Edited".into()), ..Default::default() }).await.unwrap();
        ship(&a, "pc-a", &b);
        ship(&b, "pc-b", &a);

        assert!(a.list().await.unwrap().is_empty());
        assert!(b.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn applying_a_batch_twice_changes_nothing() {
        let (a, b) = pair().await;
        a.create(new("Once")).await.unwrap();
        let ops: Vec<Op> = a.pending_ops(100).unwrap().into_iter().map(|(_, op)| op).collect();
        assert_eq!(b.apply_ops("pc-a", 1, &ops).unwrap(), 1);
        assert_eq!(b.apply_ops("pc-a", 1, &ops).unwrap(), 0);
        assert_eq!(b.cursor("pc-a").unwrap(), 1);
    }

    #[tokio::test]
    async fn enabling_sync_queues_what_was_already_there() {
        let db = SqliteDb::open_in_memory().unwrap();
        db.create(new("Before sync")).await.unwrap();
        assert!(db.pending_ops(10).unwrap().is_empty(), "nothing is logged while sync is off");

        db.enable_sync("pc-a").unwrap();
        assert_eq!(db.pending_ops(10).unwrap().len(), 1);
        db.enable_sync("pc-a").unwrap();
        assert_eq!(db.pending_ops(10).unwrap().len(), 1, "enabling again doesn't queue twice");

        db.disable_sync().unwrap();
        assert!(db.pending_ops(10).unwrap().is_empty());
        assert_eq!(db.sync_device().unwrap(), None);
    }

    #[tokio::test]
    async fn resetting_requeues_everything_for_a_new_location() {
        let (a, b) = pair().await;
        a.create(new("Already uploaded")).await.unwrap();
        ship(&a, "pc-a", &b);
        drain(&a);
        assert_eq!((a.last_segment().unwrap(), b.cursor("pc-a").unwrap()), (1, 1));

        a.reset_sync().unwrap();
        b.reset_sync().unwrap();
        assert_eq!((a.last_segment().unwrap(), b.cursor("pc-a").unwrap()), (0, 0));
        a.enable_sync("pc-a").unwrap();
        assert_eq!(a.pending_ops(10).unwrap().len(), 1, "the task goes up again to the new location");
    }

    #[tokio::test]
    async fn stopping_a_task_reaches_the_other_device() {
        let (a, b) = pair().await;
        let task = a.create(new("Focus")).await.unwrap();
        a.start(PROVIDER_KEY, &task.id, &"pc-a".into()).await.unwrap();
        ship(&a, "pc-a", &b);
        drain(&a);

        a.stop(PROVIDER_KEY, &task.id).await.unwrap();
        ship(&a, "pc-a", &b);
        assert!(b.focus().await.unwrap().is_empty());
    }
}
