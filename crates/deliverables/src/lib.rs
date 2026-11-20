mod sync;

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use multitasker_core::{Error, Result, TaskId};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use sync::{sync, SyncReport};

/// Copies bigger than this stay on the computer they were handed in on, so sync rounds stay quick.
pub const MAX_SYNC_SIZE: u64 = 100 * 1024 * 1024;

const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS deliverables (
        id          TEXT PRIMARY KEY,
        provider    TEXT NOT NULL,
        task_id     TEXT NOT NULL,
        name        TEXT NOT NULL,
        created_at  TEXT NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS deliverables_by_name ON deliverables (provider, task_id, name COLLATE NOCASE);
    CREATE TABLE IF NOT EXISTS revisions (
        id              TEXT PRIMARY KEY,
        deliverable_id  TEXT NOT NULL REFERENCES deliverables (id) ON DELETE CASCADE,
        number          INTEGER NOT NULL,
        sha256          TEXT NOT NULL,
        size            INTEGER NOT NULL,
        source          TEXT NOT NULL,
        device          TEXT,
        added_at        TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS revisions_by_deliverable ON revisions (deliverable_id);
    CREATE TABLE IF NOT EXISTS removals (
        provider    TEXT NOT NULL,
        task_id     TEXT NOT NULL,
        name        TEXT NOT NULL,
        removed_at  TEXT NOT NULL,
        device      TEXT,
        PRIMARY KEY (provider, task_id, name)
    );
"#;

/// A piece of work handed in for a task, known by its file name, with every version handed in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Deliverable {
    pub id: String,
    pub task_id: TaskId,
    pub name: String,
    /// Newest first.
    pub revisions: Vec<Revision>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Revision {
    pub id: String,
    /// Its place among the versions, oldest first, wherever each was handed in.
    pub number: u32,
    pub size: u64,
    pub sha256: String,
    /// Where the file was handed in from.
    pub source: String,
    pub device: Option<String>,
    pub added_at: DateTime<Utc>,
    /// The copy is on this computer; one handed in elsewhere arrives with sync.
    pub available: bool,
}

/// What handing in one file did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandedIn {
    pub name: String,
    pub number: u32,
    /// The file matched the latest revision, so nothing new was kept.
    pub unchanged: bool,
}

/// A revision as other computers learn of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub provider: String,
    pub task_id: TaskId,
    pub name: String,
    pub sha256: String,
    pub size: u64,
    pub source: String,
    pub device: Option<String>,
    pub added_at: DateTime<Utc>,
}

/// Deleted work: everything handed in up to `removed_at` for that task, or only the deliverable with `name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Removal {
    pub provider: String,
    pub task_id: TaskId,
    /// `None` for the whole task.
    pub name: Option<String>,
    pub removed_at: DateTime<Utc>,
    pub device: Option<String>,
}

/// Deliverables on this computer: a small database of revisions and a folder of the copies themselves.
pub struct Store {
    dir: PathBuf,
    conn: Mutex<Connection>,
}

fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}

fn time(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw).map(|t| t.with_timezone(&Utc)).unwrap_or_default()
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("copies")).map_err(storage)?;
        let conn = Connection::open(dir.join("deliverables.db")).map_err(storage)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;").map_err(storage)?;
        conn.execute_batch(SCHEMA).map_err(storage)?;
        Ok(Self { dir, conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn copy_path(&self, sha256: &str) -> PathBuf {
        self.dir.join("copies").join(&sha256[..2.min(sha256.len())]).join(sha256)
    }

    fn has_copy(&self, sha256: &str) -> bool {
        sha256.len() == 64 && self.copy_path(sha256).is_file()
    }

    /// Keeps a copy of a file as the next revision of the task's deliverable with that name.
    pub fn hand_in(&self, provider: &str, task_id: &str, path: &Path, device: Option<&str>) -> Result<HandedIn> {
        let meta = fs::metadata(path).map_err(|e| Error::Invalid(format!("Can't read {}: {e}", path.display())))?;
        if meta.is_dir() {
            return Err(Error::Invalid(format!(
                "{} is a folder; hand in the files inside it, or a zip of it.",
                path.display()
            )));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Error::Invalid(format!("{} has no file name.", path.display())))?;
        let (sha256, size) = self.keep_copy(path)?;

        let conn = self.conn();
        let latest: Option<(String, i64)> = conn
            .query_row(
                "SELECT r.sha256, (SELECT COUNT(*) FROM revisions WHERE deliverable_id = d.id)
                 FROM deliverables d JOIN revisions r ON r.deliverable_id = d.id
                 WHERE d.provider = ?1 AND d.task_id = ?2 AND d.name = ?3 COLLATE NOCASE
                 ORDER BY r.added_at DESC, r.id DESC LIMIT 1",
                params![provider, task_id, name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        if let Some((hash, count)) = &latest {
            if *hash == sha256 {
                return Ok(HandedIn { name, number: *count as u32, unchanged: true });
            }
        }
        let record = Record {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.into(),
            task_id: task_id.into(),
            name: name.clone(),
            sha256,
            size,
            source: path.display().to_string(),
            device: device.map(str::to_owned),
            added_at: Utc::now(),
        };
        insert(&conn, &record)?;
        Ok(HandedIn { name, number: latest.map_or(1, |(_, n)| n as u32 + 1), unchanged: false })
    }

    /// Copies a file in under its hash, so the same content is only ever kept once.
    fn keep_copy(&self, path: &Path) -> Result<(String, u64)> {
        let mut source = File::open(path).map_err(|e| Error::Invalid(format!("Can't open {}: {e}", path.display())))?;
        let temp = self.dir.join("copies").join(format!("incoming-{}", uuid::Uuid::new_v4()));
        let mut out = File::create(&temp).map_err(storage)?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut buf = vec![0u8; 256 * 1024];
        let copied = (|| -> std::io::Result<()> {
            loop {
                let n = source.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n])?;
                size += n as u64;
            }
            out.sync_all()
        })();
        drop(out);
        if let Err(e) = copied {
            let _ = fs::remove_file(&temp);
            return Err(Error::Storage(format!("Couldn't copy {}: {e}", path.display())));
        }
        let sha256 = hex::encode(hasher.finalize());
        self.settle(&temp, &sha256)?;
        Ok((sha256, size))
    }

    /// Moves a finished temporary file into place under its hash.
    fn settle(&self, temp: &Path, sha256: &str) -> Result<()> {
        let target = self.copy_path(sha256);
        if target.exists() {
            let _ = fs::remove_file(temp);
        } else {
            fs::create_dir_all(target.parent().expect("copies live in a subfolder")).map_err(storage)?;
            fs::rename(temp, &target).map_err(storage)?;
            make_read_only(&target);
        }
        Ok(())
    }

    /// Keeps a copy that arrived through sync, if it really is the content its hash names.
    pub fn receive_copy(&self, sha256: &str, bytes: &[u8]) -> Result<bool> {
        if hex::encode(Sha256::digest(bytes)) != sha256 {
            return Ok(false);
        }
        let temp = self.dir.join("copies").join(format!("incoming-{}", uuid::Uuid::new_v4()));
        fs::write(&temp, bytes).map_err(storage)?;
        self.settle(&temp, sha256)?;
        Ok(true)
    }

    pub fn read_copy(&self, sha256: &str) -> Result<Vec<u8>> {
        fs::read(self.copy_path(sha256)).map_err(storage)
    }

    pub fn list(&self, provider: &str, task_id: &str) -> Result<Vec<Deliverable>> {
        let rows: Vec<(String, String, Record)> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT d.id, d.name, r.id, r.sha256, r.size, r.source, r.device, r.added_at
                     FROM deliverables d JOIN revisions r ON r.deliverable_id = d.id
                     WHERE d.provider = ?1 AND d.task_id = ?2
                     ORDER BY d.created_at, d.name, r.added_at, r.id",
                )
                .map_err(storage)?;
            let rows = stmt
                .query_map(params![provider, task_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        Record {
                            id: r.get(2)?,
                            provider: provider.into(),
                            task_id: task_id.into(),
                            name: r.get(1)?,
                            sha256: r.get(3)?,
                            size: r.get::<_, i64>(4)? as u64,
                            source: r.get(5)?,
                            device: r.get(6)?,
                            added_at: time(&r.get::<_, String>(7)?),
                        },
                    ))
                })
                .map_err(storage)?;
            rows.collect::<rusqlite::Result<_>>().map_err(storage)?
        };
        let mut out: Vec<Deliverable> = Vec::new();
        for (id, name, record) in rows {
            if out.last().is_none_or(|d| d.id != id) {
                out.push(Deliverable { id: id.clone(), task_id: task_id.into(), name, revisions: Vec::new() });
            }
            let d = out.last_mut().expect("just pushed");
            d.revisions.push(Revision {
                number: d.revisions.len() as u32 + 1,
                available: self.has_copy(&record.sha256),
                id: record.id,
                size: record.size,
                sha256: record.sha256,
                source: record.source,
                device: record.device,
                added_at: record.added_at,
            });
        }
        for d in &mut out {
            d.revisions.reverse();
        }
        Ok(out)
    }

    /// How many deliverables each of a provider's tasks has.
    pub fn counts(&self, provider: &str) -> Result<HashMap<TaskId, u32>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT task_id, COUNT(*) FROM deliverables WHERE provider = ?1 GROUP BY task_id")
            .map_err(storage)?;
        let rows = stmt.query_map(params![provider], |r| Ok((r.get(0)?, r.get(1)?))).map_err(storage)?;
        rows.collect::<rusqlite::Result<_>>().map_err(storage)
    }

    fn revision(&self, id: &str) -> Result<(String, String)> {
        let (name, sha256): (String, String) = self
            .conn()
            .query_row(
                "SELECT d.name, r.sha256 FROM revisions r JOIN deliverables d ON d.id = r.deliverable_id WHERE r.id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| Error::NotFound("That revision no longer exists.".into()))?;
        if !self.has_copy(&sha256) {
            return Err(Error::Invalid(
                "That version was handed in on another computer and hasn't arrived here yet.".into(),
            ));
        }
        Ok((name, sha256))
    }

    /// A read-only copy of a revision under its own name, to open without touching the kept one.
    pub fn open_copy(&self, revision_id: &str) -> Result<PathBuf> {
        let (name, sha256) = self.revision(revision_id)?;
        let folder = self.dir.join("opened").join(revision_id);
        fs::create_dir_all(&folder).map_err(storage)?;
        let target = folder.join(&name);
        if !target.exists() {
            fs::copy(self.copy_path(&sha256), &target).map_err(storage)?;
            make_read_only(&target);
        }
        Ok(target)
    }

    /// Saves a revision wherever the person chose.
    pub fn export(&self, revision_id: &str, dest: &Path) -> Result<()> {
        let (_, sha256) = self.revision(revision_id)?;
        fs::copy(self.copy_path(&sha256), dest).map_err(|e| Error::Storage(format!("Couldn't save it there: {e}")))?;
        // The kept copy is read-only; the saved one is the person's to edit.
        make_writable(dest);
        Ok(())
    }

    /// Deletes a deliverable and every revision of it, here and, through sync, everywhere.
    pub fn remove(&self, deliverable_id: &str, device: Option<&str>) -> Result<()> {
        let found: Option<(String, String, String)> = self
            .conn()
            .query_row(
                "SELECT provider, task_id, name FROM deliverables WHERE id = ?1",
                params![deliverable_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(storage)?;
        let Some((provider, task_id, name)) = found else { return Ok(()) };
        self.apply_removal(&Removal { provider, task_id, name: Some(name), removed_at: Utc::now(), device: device.map(str::to_owned) })
    }

    /// Drops a deleted task's deliverables.
    pub fn forget_task(&self, provider: &str, task_id: &str, device: Option<&str>) -> Result<()> {
        self.apply_removal(&Removal {
            provider: provider.into(),
            task_id: task_id.into(),
            name: None,
            removed_at: Utc::now(),
            device: device.map(str::to_owned),
        })
    }

    fn apply_removal(&self, removal: &Removal) -> Result<()> {
        {
            let conn = self.conn();
            let name = removal.name.clone().unwrap_or_default();
            conn.execute(
                "INSERT INTO removals (provider, task_id, name, removed_at, device) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (provider, task_id, name) DO UPDATE SET removed_at = MAX(removed_at, excluded.removed_at)",
                params![removal.provider, removal.task_id, name, removal.removed_at.to_rfc3339(), removal.device],
            )
            .map_err(storage)?;
            conn.execute(
                "DELETE FROM revisions WHERE added_at <= ?4 AND deliverable_id IN (
                     SELECT id FROM deliverables WHERE provider = ?1 AND task_id = ?2 AND (?3 = '' OR name = ?3 COLLATE NOCASE))",
                params![removal.provider, removal.task_id, name, removal.removed_at.to_rfc3339()],
            )
            .map_err(storage)?;
            conn.execute("DELETE FROM deliverables WHERE id NOT IN (SELECT deliverable_id FROM revisions)", [])
                .map_err(storage)?;
        }
        self.sweep()
    }

    /// What this computer tells the others: the work handed in here and what was deleted here.
    pub fn own_records(&self, device: &str) -> Result<(Vec<Record>, Vec<Removal>)> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT r.id, d.provider, d.task_id, d.name, r.sha256, r.size, r.source, r.device, r.added_at
                 FROM revisions r JOIN deliverables d ON d.id = r.deliverable_id WHERE r.device = ?1",
            )
            .map_err(storage)?;
        let records = stmt
            .query_map(params![device], |r| {
                Ok(Record {
                    id: r.get(0)?,
                    provider: r.get(1)?,
                    task_id: r.get(2)?,
                    name: r.get(3)?,
                    sha256: r.get(4)?,
                    size: r.get::<_, i64>(5)? as u64,
                    source: r.get(6)?,
                    device: r.get(7)?,
                    added_at: time(&r.get::<_, String>(8)?),
                })
            })
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?;
        let mut stmt = conn
            .prepare("SELECT provider, task_id, name, removed_at, device FROM removals WHERE device = ?1")
            .map_err(storage)?;
        let removals = stmt
            .query_map(params![device], |r| {
                let name: String = r.get(2)?;
                Ok(Removal {
                    provider: r.get(0)?,
                    task_id: r.get(1)?,
                    name: (!name.is_empty()).then_some(name),
                    removed_at: time(&r.get::<_, String>(3)?),
                    device: r.get(4)?,
                })
            })
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?;
        Ok((records, removals))
    }

    /// Takes in what another computer handed in and deleted; returns how many new revisions arrived.
    pub fn merge(&self, records: &[Record], removals: &[Removal]) -> Result<usize> {
        for removal in removals {
            self.apply_removal(removal)?;
        }
        let conn = self.conn();
        let known: HashSet<String> = {
            let mut stmt = conn.prepare("SELECT id FROM revisions").map_err(storage)?;
            let ids = stmt.query_map([], |r| r.get(0)).map_err(storage)?;
            ids.collect::<rusqlite::Result<_>>().map_err(storage)?
        };
        let mut added = 0;
        for record in records.iter().filter(|r| !known.contains(&r.id)) {
            let removed_at: Option<String> = conn
                .query_row(
                    "SELECT MAX(removed_at) FROM removals WHERE provider = ?1 AND task_id = ?2 AND (name = '' OR name = ?3 COLLATE NOCASE)",
                    params![record.provider, record.task_id, record.name],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if removed_at.is_some_and(|at| time(&at) >= record.added_at) {
                continue;
            }
            insert(&conn, record)?;
            added += 1;
        }
        Ok(added)
    }

    /// Every revision's content, by hash, with its size.
    pub fn all_copies(&self) -> Result<Vec<(String, u64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT sha256, size FROM revisions").map_err(storage)?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))).map_err(storage)?;
        rows.collect::<rusqlite::Result<_>>().map_err(storage)
    }

    pub fn has(&self, sha256: &str) -> bool {
        self.has_copy(sha256)
    }

    /// Deletes copies no revision refers to any more.
    fn sweep(&self) -> Result<()> {
        let kept: HashSet<String> = self.all_copies()?.into_iter().map(|(sha, _)| sha).collect();
        let Ok(folders) = fs::read_dir(self.dir.join("copies")) else { return Ok(()) };
        for folder in folders.flatten().filter(|f| f.path().is_dir()) {
            for file in fs::read_dir(folder.path()).into_iter().flatten().flatten() {
                if !kept.contains(&file.file_name().to_string_lossy().into_owned()) {
                    make_writable(&file.path());
                    let _ = fs::remove_file(file.path());
                }
            }
        }
        Ok(())
    }
}

/// Adds a revision, under the deliverable of that name for its task (made if it's the first).
fn insert(conn: &Connection, record: &Record) -> Result<()> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM deliverables WHERE provider = ?1 AND task_id = ?2 AND name = ?3 COLLATE NOCASE",
            params![record.provider, record.task_id, record.name],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    let deliverable = match existing {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO deliverables (id, provider, task_id, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, record.provider, record.task_id, record.name, record.added_at.to_rfc3339()],
            )
            .map_err(storage)?;
            id
        }
    };
    conn.execute(
        "INSERT INTO revisions (id, deliverable_id, number, sha256, size, source, device, added_at)
         VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7)",
        params![
            record.id,
            deliverable,
            record.sha256,
            record.size as i64,
            record.source,
            record.device,
            record.added_at.to_rfc3339()
        ],
    )
    .map_err(storage)?;
    Ok(())
}

/// Kept copies are read-only, so opening one by mistake can't change what was handed in.
fn make_read_only(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut permissions = meta.permissions();
        permissions.set_readonly(true);
        let _ = fs::set_permissions(path, permissions);
    }
}

fn make_writable(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut permissions = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("multitasker-deliverables-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub fn file(&self, name: &str, content: &str) -> PathBuf {
            let path = self.0.join("work").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            make_writable(&path);
            fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for entry in walk(&self.0) {
                make_writable(&entry);
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            }
            out.push(p);
        }
        out
    }

    #[test]
    fn each_changed_hand_in_is_a_new_revision() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.0.join("store")).unwrap();
        let essay = scratch.file("Essay.docx", "draft");
        assert_eq!(store.hand_in("sqlite", "t1", &essay, Some("pc-1")).unwrap().number, 1);
        let again = store.hand_in("sqlite", "t1", &essay, Some("pc-1")).unwrap();
        assert!(again.unchanged && again.number == 1, "the same content isn't kept twice");

        scratch.file("Essay.docx", "final");
        assert_eq!(store.hand_in("sqlite", "t1", &essay, Some("pc-1")).unwrap().number, 2);

        let list = store.list("sqlite", "t1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].revisions.iter().map(|r| r.number).collect::<Vec<_>>(), [2, 1]);
        assert!(list[0].revisions.iter().all(|r| r.available));
        assert!(store.list("sqlite", "t2").unwrap().is_empty());
        assert_eq!(store.counts("sqlite").unwrap().get("t1"), Some(&1));
    }

    #[test]
    fn kept_copies_survive_the_original_changing() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.0.join("store")).unwrap();
        let notes = scratch.file("notes.txt", "version one");
        store.hand_in("sqlite", "t1", &notes, None).unwrap();
        scratch.file("notes.txt", "overwritten");
        fs::remove_file(&notes).unwrap();

        let first = store.list("sqlite", "t1").unwrap()[0].revisions[0].id.clone();
        let opened = store.open_copy(&first).unwrap();
        assert_eq!(fs::read_to_string(&opened).unwrap(), "version one");
        assert!(fs::metadata(&opened).unwrap().permissions().readonly());

        let saved = scratch.0.join("saved.txt");
        store.export(&first, &saved).unwrap();
        assert!(!fs::metadata(&saved).unwrap().permissions().readonly(), "a saved copy can be edited");
    }

    #[test]
    fn folders_are_refused_and_removed_work_is_cleaned_up() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.0.join("store")).unwrap();
        assert!(store.hand_in("sqlite", "t1", &scratch.0, None).is_err());

        let a = scratch.file("a.txt", "shared");
        let b = scratch.file("b.txt", "shared");
        store.hand_in("sqlite", "t1", &a, None).unwrap();
        store.hand_in("sqlite", "t2", &b, None).unwrap();
        let copies = || walk(&scratch.0.join("store").join("copies")).into_iter().filter(|p| p.is_file()).count();
        assert_eq!(copies(), 1, "identical content is kept once");

        store.forget_task("sqlite", "t1", None).unwrap();
        assert_eq!(copies(), 1, "still needed by t2");
        let id = store.list("sqlite", "t2").unwrap()[0].id.clone();
        store.remove(&id, None).unwrap();
        assert_eq!(copies(), 0);
    }
}
