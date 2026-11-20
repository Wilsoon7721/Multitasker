use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use multitasker_core::{Error, Result, TaskId};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

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
    pub number: u32,
    pub size: u64,
    pub sha256: String,
    /// Where the file was handed in from.
    pub source: String,
    pub device: Option<String>,
    pub added_at: DateTime<Utc>,
}

/// What handing in one file did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandedIn {
    pub name: String,
    pub number: u32,
    /// The file matched the latest revision, so nothing new was kept.
    pub unchanged: bool,
}

/// Deliverables on this computer: a small database of revisions and a folder of the copies themselves.
pub struct Store {
    dir: PathBuf,
    conn: Mutex<Connection>,
}

fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
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
        self.dir.join("copies").join(&sha256[..2]).join(sha256)
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
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM deliverables WHERE provider = ?1 AND task_id = ?2 AND name = ?3 COLLATE NOCASE",
                params![provider, task_id, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        let now = Utc::now().to_rfc3339();
        let deliverable = match existing {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO deliverables (id, provider, task_id, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, provider, task_id, name, now],
                )
                .map_err(storage)?;
                id
            }
        };
        let latest: Option<(u32, String)> = conn
            .query_row(
                "SELECT number, sha256 FROM revisions WHERE deliverable_id = ?1 ORDER BY number DESC LIMIT 1",
                params![deliverable],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        if let Some((number, hash)) = &latest {
            if *hash == sha256 {
                return Ok(HandedIn { name, number: *number, unchanged: true });
            }
        }
        let number = latest.map_or(1, |(n, _)| n + 1);
        conn.execute(
            "INSERT INTO revisions (id, deliverable_id, number, sha256, size, source, device, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                uuid::Uuid::new_v4().to_string(),
                deliverable,
                number,
                sha256,
                size as i64,
                path.display().to_string(),
                device,
                now
            ],
        )
        .map_err(storage)?;
        Ok(HandedIn { name, number, unchanged: false })
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
        let target = self.copy_path(&sha256);
        if target.exists() {
            let _ = fs::remove_file(&temp);
        } else {
            fs::create_dir_all(target.parent().expect("copies live in a subfolder")).map_err(storage)?;
            fs::rename(&temp, &target).map_err(storage)?;
            make_read_only(&target);
        }
        Ok((sha256, size))
    }

    pub fn list(&self, provider: &str, task_id: &str) -> Result<Vec<Deliverable>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT d.id, d.name, r.id, r.number, r.sha256, r.size, r.source, r.device, r.added_at
                 FROM deliverables d JOIN revisions r ON r.deliverable_id = d.id
                 WHERE d.provider = ?1 AND d.task_id = ?2
                 ORDER BY d.created_at, d.name, r.number DESC",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map(params![provider, task_id], |r| {
                let added: String = r.get(8)?;
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    Revision {
                        id: r.get(2)?,
                        number: r.get(3)?,
                        sha256: r.get(4)?,
                        size: r.get::<_, i64>(5)? as u64,
                        source: r.get(6)?,
                        device: r.get(7)?,
                        added_at: DateTime::parse_from_rfc3339(&added).map(|t| t.with_timezone(&Utc)).unwrap_or_default(),
                    },
                ))
            })
            .map_err(storage)?;
        let mut out: Vec<Deliverable> = Vec::new();
        for row in rows {
            let (id, name, revision) = row.map_err(storage)?;
            match out.last_mut() {
                Some(d) if d.id == id => d.revisions.push(revision),
                _ => out.push(Deliverable { id, task_id: task_id.into(), name, revisions: vec![revision] }),
            }
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
        self.conn()
            .query_row(
                "SELECT d.name, r.sha256 FROM revisions r JOIN deliverables d ON d.id = r.deliverable_id WHERE r.id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| Error::NotFound("That revision no longer exists.".into()))
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
        if let Ok(meta) = fs::metadata(dest) {
            let mut permissions = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(dest, permissions);
        }
        Ok(())
    }

    pub fn remove(&self, deliverable_id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM deliverables WHERE id = ?1", params![deliverable_id]).map_err(storage)?;
        self.sweep()
    }

    /// Drops a deleted task's deliverables.
    pub fn forget_task(&self, provider: &str, task_id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM deliverables WHERE provider = ?1 AND task_id = ?2", params![provider, task_id])
            .map_err(storage)?;
        self.sweep()
    }

    /// Deletes copies no revision refers to any more.
    fn sweep(&self) -> Result<()> {
        let kept: std::collections::HashSet<String> = {
            let conn = self.conn();
            let mut stmt = conn.prepare("SELECT DISTINCT sha256 FROM revisions").map_err(storage)?;
            let rows = stmt.query_map([], |r| r.get(0)).map_err(storage)?;
            rows.collect::<rusqlite::Result<_>>().map_err(storage)?
        };
        let Ok(folders) = fs::read_dir(self.dir.join("copies")) else { return Ok(()) };
        for folder in folders.flatten().filter(|f| f.path().is_dir()) {
            for file in fs::read_dir(folder.path()).into_iter().flatten().flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                if !kept.contains(&name) {
                    let path = file.path();
                    let mut permissions = fs::metadata(&path).map_err(storage)?.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    let _ = fs::set_permissions(&path, permissions);
                    let _ = fs::remove_file(path);
                }
            }
        }
        Ok(())
    }
}

/// Kept copies are read-only, so opening one by mistake can't change what was handed in.
fn make_read_only(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut permissions = meta.permissions();
        permissions.set_readonly(true);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("multitasker-deliverables-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, content: &str) -> PathBuf {
            let path = self.0.join("work").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if path.exists() {
                make_writable(&path);
            }
            fs::write(&path, content).unwrap();
            path
        }
    }

    fn make_writable(path: &Path) {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions).unwrap();
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for entry in walk(&self.0) {
                let _ = fs::metadata(&entry).map(|m| {
                    let mut p = m.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    p.set_readonly(false);
                    let _ = fs::set_permissions(&entry, p);
                });
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
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

        store.forget_task("sqlite", "t1").unwrap();
        assert_eq!(copies(), 1, "still needed by t2");
        let id = store.list("sqlite", "t2").unwrap()[0].id.clone();
        store.remove(&id).unwrap();
        assert_eq!(copies(), 0);
    }
}
