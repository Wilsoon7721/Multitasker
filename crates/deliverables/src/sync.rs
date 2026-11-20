use std::collections::HashSet;

use multitasker_sync::{Remote, RemoteError};
use serde::{Deserialize, Serialize};

use crate::{Record, Removal, Store, MAX_SYNC_SIZE};

const RECORDS: &str = "multitasker/v1/deliverables/";
const COPIES: &str = "multitasker/v1/copies/";

/// One computer's list of what it handed in and deleted; each computer only ever writes its own.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    device: String,
    #[serde(default)]
    records: Vec<Record>,
    #[serde(default)]
    removals: Vec<Removal>,
}

/// What one round of syncing handed-in work did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SyncReport {
    /// Versions handed in elsewhere that this computer now knows of.
    pub learned: usize,
    pub uploaded: usize,
    pub downloaded: usize,
}

/// Swaps lists of handed-in work with the other computers, then the copies themselves.
pub fn sync(store: &Store, remote: &dyn Remote, me: &str) -> Result<SyncReport, SyncFailure> {
    let mut report = SyncReport::default();

    // Its own list is read too, so a reinstall gets back the work it handed in before.
    for key in remote.list(RECORDS)?.into_iter().filter(|k| k.ends_with(".json")) {
        let Some(bytes) = remote.get(&key)? else { continue };
        // A list that doesn't parse is still arriving through the sync client; it's read next round.
        let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else { continue };
        report.learned += store.merge(&manifest.records, &manifest.removals)?;
    }

    let (records, removals) = store.own_records(me)?;
    let manifest = Manifest { device: me.to_owned(), records, removals };
    remote.put(&format!("{RECORDS}{me}.json"), &serde_json::to_vec(&manifest).expect("records always serialize"))?;

    let there: HashSet<String> =
        remote.list(COPIES)?.into_iter().filter_map(|k| k.strip_prefix(COPIES).map(str::to_owned)).collect();
    for (sha256, size) in store.all_copies()? {
        if size > MAX_SYNC_SIZE || sha256.len() != 64 {
            continue;
        }
        let here = store.has(&sha256);
        if here && !there.contains(&sha256) {
            remote.put(&format!("{COPIES}{sha256}"), &store.read_copy(&sha256)?)?;
            report.uploaded += 1;
        } else if !here && there.contains(&sha256) {
            if let Some(bytes) = remote.get(&format!("{COPIES}{sha256}"))? {
                // A copy that's still uploading doesn't match its hash yet; it's fetched again next round.
                if store.receive_copy(&sha256, &bytes)? {
                    report.downloaded += 1;
                }
            }
        }
    }
    Ok(report)
}

#[derive(Debug, thiserror::Error)]
pub enum SyncFailure {
    #[error("Couldn't sync handed-in work: {0}")]
    Remote(#[from] RemoteError),
    #[error("{0}")]
    Local(#[from] multitasker_core::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::Scratch;
    use multitasker_sync::FolderRemote;

    #[test]
    fn work_handed_in_on_one_computer_reaches_another() {
        let scratch = Scratch::new();
        let shared = scratch.0.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let remote = FolderRemote::new(&shared).unwrap();
        let laptop = Store::open(scratch.0.join("laptop")).unwrap();
        let desktop = Store::open(scratch.0.join("desktop")).unwrap();

        let essay = scratch.file("Essay.docx", "draft");
        laptop.hand_in("sqlite", "t1", &essay, Some("laptop")).unwrap();
        let first = sync(&laptop, &remote, "laptop").unwrap();
        assert_eq!((first.learned, first.uploaded), (0, 1));

        let arrived = sync(&desktop, &remote, "desktop").unwrap();
        assert_eq!((arrived.learned, arrived.downloaded), (1, 1));
        let list = desktop.list("sqlite", "t1").unwrap();
        assert!(list[0].revisions[0].available);
        let opened = desktop.open_copy(&list[0].revisions[0].id).unwrap();
        assert_eq!(std::fs::read_to_string(opened).unwrap(), "draft");

        // A second version handed in on the desktop stacks on the laptop's first.
        scratch.file("Essay.docx", "final");
        desktop.hand_in("sqlite", "t1", &essay, Some("desktop")).unwrap();
        sync(&desktop, &remote, "desktop").unwrap();
        sync(&laptop, &remote, "laptop").unwrap();
        let versions = laptop.list("sqlite", "t1").unwrap()[0].revisions.iter().map(|r| r.number).collect::<Vec<_>>();
        assert_eq!(versions, [2, 1]);

        let quiet = sync(&laptop, &remote, "laptop").unwrap();
        assert_eq!(quiet, SyncReport::default(), "nothing new means nothing moves");
    }

    #[test]
    fn deleting_on_one_computer_deletes_everywhere_but_keeps_later_work() {
        let scratch = Scratch::new();
        let shared = scratch.0.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let remote = FolderRemote::new(&shared).unwrap();
        let laptop = Store::open(scratch.0.join("laptop")).unwrap();
        let desktop = Store::open(scratch.0.join("desktop")).unwrap();

        laptop.hand_in("sqlite", "t1", &scratch.file("a.txt", "one"), Some("laptop")).unwrap();
        sync(&laptop, &remote, "laptop").unwrap();
        sync(&desktop, &remote, "desktop").unwrap();

        let id = desktop.list("sqlite", "t1").unwrap()[0].id.clone();
        desktop.remove(&id, Some("desktop")).unwrap();
        sync(&desktop, &remote, "desktop").unwrap();
        sync(&laptop, &remote, "laptop").unwrap();
        assert!(laptop.list("sqlite", "t1").unwrap().is_empty(), "deleted on the laptop too");

        std::thread::sleep(std::time::Duration::from_millis(5));
        laptop.hand_in("sqlite", "t1", &scratch.file("a.txt", "two"), Some("laptop")).unwrap();
        sync(&laptop, &remote, "laptop").unwrap();
        sync(&desktop, &remote, "desktop").unwrap();
        assert_eq!(desktop.list("sqlite", "t1").unwrap().len(), 1, "work handed in after the deletion survives it");
    }

    #[test]
    fn a_reinstall_gets_its_own_work_back() {
        let scratch = Scratch::new();
        let shared = scratch.0.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let remote = FolderRemote::new(&shared).unwrap();
        let old = Store::open(scratch.0.join("old")).unwrap();
        old.hand_in("sqlite", "t1", &scratch.file("a.txt", "mine"), Some("pc")).unwrap();
        sync(&old, &remote, "pc").unwrap();

        let fresh = Store::open(scratch.0.join("fresh")).unwrap();
        sync(&fresh, &remote, "pc").unwrap();
        assert_eq!(fresh.list("sqlite", "t1").unwrap().len(), 1);
        let manifest: Manifest = serde_json::from_slice(&remote.get("multitasker/v1/deliverables/pc.json").unwrap().unwrap()).unwrap();
        assert_eq!(manifest.records.len(), 1, "its list still names that work");
    }
}
