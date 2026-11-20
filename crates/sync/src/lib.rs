mod remote;

use std::collections::BTreeMap;

use chrono::Utc;
use multitasker_core::{DeviceInfo, Op};
use multitasker_provider_sqlite::SqliteDb;
use thiserror::Error;

pub use remote::{FolderRemote, Remote, RemoteError};

/// Everything MultiTasker writes lives under this key, so a shared folder can hold other files too.
const ROOT: &str = "multitasker/v1";
/// At most this many changes go in one uploaded batch.
const BATCH: usize = 500;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Couldn't reach the sync location: {0}")]
    Remote(#[from] RemoteError),
    #[error("{0}")]
    Local(#[from] multitasker_core::Error),
}

/// What one sync round did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub uploaded: usize,
    pub applied: usize,
    pub devices: Vec<DeviceInfo>,
}

/// Downloads and merges other devices' new changes, then uploads this device's own; devices only write their own files.
pub fn sync_once(db: &SqliteDb, remote: &dyn Remote, me: &DeviceInfo) -> Result<Report, SyncError> {
    db.enable_sync(&me.id)?;
    let mut report = Report::default();

    let batches = batches_by_device(remote.list(&format!("{ROOT}/ops/"))?);
    for (device, segments) in &batches {
        // A reinstall keeps its device id but loses its database, so this device's own batches are read too.
        let seen = if device == &me.id { db.cursor(device)?.max(db.last_segment()?) } else { db.cursor(device)? };
        for &segment in segments.iter().filter(|&&s| s > seen) {
            let Some(bytes) = remote.get(&segment_key(device, segment))? else { break };
            // A batch that doesn't parse is still arriving through the sync client; it's retried next round.
            let Ok(ops) = serde_json::from_slice::<Vec<Op>>(&bytes) else { break };
            report.applied += db.apply_ops(device, segment, &ops)?;
        }
    }

    let mut next = db.last_segment()?.max(batches.get(&me.id).and_then(|s| s.last().copied()).unwrap_or(0)) + 1;
    loop {
        let pending = db.pending_ops(BATCH)?;
        let Some(&(through, _)) = pending.last() else { break };
        let ops: Vec<&Op> = pending.iter().map(|(_, op)| op).collect();
        remote.put(&segment_key(&me.id, next), &to_json(&ops))?;
        db.ack_ops(through, next)?;
        report.uploaded += ops.len();
        next += 1;
    }

    let me_now = DeviceInfo { last_seen: Utc::now(), ..me.clone() };
    remote.put(&format!("{ROOT}/devices/{}.json", me.id), &to_json(&me_now))?;
    for key in remote.list(&format!("{ROOT}/devices/"))? {
        let parsed = remote.get(&key)?.and_then(|b| serde_json::from_slice::<DeviceInfo>(&b).ok());
        report.devices.extend(parsed);
    }
    db.save_devices(&report.devices)?;
    Ok(report)
}

fn segment_key(device: &str, segment: u64) -> String {
    format!("{ROOT}/ops/{device}/{segment:010}.json")
}

fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("sync data always serializes")
}

/// Groups batch keys by device, sorted; temp files and sync-client conflict copies aren't batches.
fn batches_by_device(keys: Vec<String>) -> BTreeMap<String, Vec<u64>> {
    let mut by_device: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for key in keys {
        let Some(rest) = key.strip_prefix(&format!("{ROOT}/ops/")) else { continue };
        let Some((device, file)) = rest.split_once('/') else { continue };
        let Some(number) = file.strip_suffix(".json") else { continue };
        if number.len() == 10 && number.bytes().all(|b| b.is_ascii_digit()) {
            by_device.entry(device.to_owned()).or_default().push(number.parse().unwrap_or(0));
        }
    }
    by_device.values_mut().for_each(|s| s.sort_unstable());
    by_device
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitasker_core::{NewTask, TaskProvider};

    fn device(id: &str) -> DeviceInfo {
        DeviceInfo { id: id.into(), name: id.to_uppercase(), last_seen: Utc::now() }
    }

    async fn titles(db: &SqliteDb) -> Vec<String> {
        let mut titles: Vec<String> = db.list().await.unwrap().into_iter().map(|t| t.title).collect();
        titles.sort();
        titles
    }

    fn shared_folder() ->(std::path::PathBuf, FolderRemote) {
        let dir = std::env::temp_dir().join(format!("multitasker-sync-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let remote = FolderRemote::new(&dir).unwrap();
        (dir, remote)
    }

    #[tokio::test]
    async fn two_computers_converge_through_a_shared_folder() {
        let (dir, remote) = shared_folder();
        let (a, b) = (SqliteDb::open_in_memory().unwrap(), SqliteDb::open_in_memory().unwrap());

        a.create(NewTask { title: "Made on A".into(), ..Default::default() }).await.unwrap();
        let first = sync_once(&a, &remote, &device("pc-a")).unwrap();
        assert_eq!((first.uploaded, first.applied), (1, 0));

        b.create(NewTask { title: "Made on B".into(), ..Default::default() }).await.unwrap();
        let second = sync_once(&b, &remote, &device("pc-b")).unwrap();
        assert_eq!((second.uploaded, second.applied), (1, 1));
        assert_eq!(second.devices.len(), 2);

        sync_once(&a, &remote, &device("pc-a")).unwrap();
        assert_eq!(titles(&a).await, ["Made on A", "Made on B"]);
        assert_eq!(titles(&b).await, ["Made on A", "Made on B"]);

        let quiet = sync_once(&a, &remote, &device("pc-a")).unwrap();
        assert_eq!((quiet.uploaded, quiet.applied), (0, 0), "nothing new means nothing moves");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_reinstall_gets_its_own_tasks_back_without_overwriting_batches() {
        let (dir, remote) = shared_folder();
        let old = SqliteDb::open_in_memory().unwrap();
        old.create(NewTask { title: "Kept".into(), ..Default::default() }).await.unwrap();
        sync_once(&old, &remote, &device("pc-a")).unwrap();

        let fresh = SqliteDb::open_in_memory().unwrap();
        fresh.create(NewTask { title: "New".into(), ..Default::default() }).await.unwrap();
        let report = sync_once(&fresh, &remote, &device("pc-a")).unwrap();
        assert_eq!(report.applied, 1);

        let mut keys = remote.list("multitasker/v1/ops/pc-a/").unwrap();
        keys.sort();
        assert_eq!(keys.len(), 2, "the new batch got a new number: {keys:?}");
        assert_eq!(fresh.list().await.unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stray_files_are_not_batches() {
        let keys = [
            "multitasker/v1/ops/pc-a/0000000002.json",
            "multitasker/v1/ops/pc-a/0000000001.json",
            "multitasker/v1/ops/pc-a/0000000003.tmp-42",
            "multitasker/v1/ops/pc-a/0000000003-JAVAV6.json",
            "multitasker/v1/devices/pc-a.json",
        ];
        let found = batches_by_device(keys.iter().map(|k| k.to_string()).collect());
        assert_eq!(found.get("pc-a"), Some(&vec![1, 2]));
        assert_eq!(found.len(), 1);
    }
}
