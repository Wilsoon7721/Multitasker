use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for RemoteError {
    fn from(e: std::io::Error) -> Self {
        RemoteError::Io(e.to_string())
    }
}

/// Somewhere every device can read and write small files by key ("a/b/c.json").
pub trait Remote: Send + Sync {
    /// Every key under `prefix`, in any order.
    fn list(&self, prefix: &str) -> Result<Vec<String>, RemoteError>;
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RemoteError>;
    fn put(&self, key: &str, data: &[u8]) -> Result<(), RemoteError>;
}

/// A folder that a sync client (OneDrive, Google Drive, Dropbox) carries between computers.
pub struct FolderRemote {
    root: PathBuf,
}

impl FolderRemote {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RemoteError> {
        let root = root.into();
        if !root.is_dir() {
            return Err(RemoteError::Io(format!("The folder {} doesn't exist.", root.display())));
        }
        Ok(Self { root })
    }

    fn path(&self, key: &str) -> PathBuf {
        key.split('/').fold(self.root.clone(), |p, part| p.join(part))
    }
}

impl Remote for FolderRemote {
    fn list(&self, prefix: &str) -> Result<Vec<String>, RemoteError> {
        let mut keys = Vec::new();
        let start = self.path(prefix.trim_end_matches('/'));
        if start.is_dir() {
            walk(&start, prefix.trim_end_matches('/'), &mut keys)?;
        }
        Ok(keys)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RemoteError> {
        match std::fs::read(self.path(key)) {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // Written to a temporary name and renamed into place, so other computers never read half a file.
    fn put(&self, key: &str, data: &[u8]) -> Result<(), RemoteError> {
        let path = self.path(key);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temp, data)?;
        std::fs::rename(&temp, &path)?;
        Ok(())
    }
}

fn walk(dir: &Path, key: &str, keys: &mut Vec<String>) -> Result<(), RemoteError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = format!("{key}/{name}");
        if entry.file_type()?.is_dir() {
            walk(&entry.path(), &child, keys)?;
        } else {
            keys.push(child);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_round_trips_keys() {
        let dir = std::env::temp_dir().join(format!("multitasker-remote-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let remote = FolderRemote::new(&dir).unwrap();

        remote.put("a/b/one.json", b"1").unwrap();
        remote.put("a/two.json", b"2").unwrap();
        let mut keys = remote.list("a/").unwrap();
        keys.sort();
        assert_eq!(keys, ["a/b/one.json", "a/two.json"]);
        assert_eq!(remote.get("a/b/one.json").unwrap().as_deref(), Some(&b"1"[..]));
        assert_eq!(remote.get("a/missing.json").unwrap(), None);
        assert!(remote.list("nothing/").unwrap().is_empty());
        assert!(FolderRemote::new(dir.join("missing")).is_err());

        let _ = std::fs::remove_dir_all(dir);
    }
}
