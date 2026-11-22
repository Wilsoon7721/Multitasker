use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Stamp;

/// Settings that follow a person between computers, each stamped with its last change so the newest one wins.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedSettings {
    #[serde(default)]
    pub entries: BTreeMap<String, SharedEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedEntry {
    pub value: Value,
    pub stamp: Stamp,
}

impl SharedSettings {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key).map(|e| &e.value)
    }

    /// The newest stamp anywhere, for making the next one.
    pub fn latest(&self) -> Option<&Stamp> {
        self.entries.values().map(|e| &e.stamp).max()
    }

    /// Sets a value if `stamp` is newer than what's there; returns whether it did.
    pub fn set(&mut self, key: &str, value: Value, stamp: Stamp) -> bool {
        match self.entries.get(key) {
            Some(kept) if kept.stamp >= stamp => false,
            _ => {
                self.entries.insert(key.to_owned(), SharedEntry { value, stamp });
                true
            }
        }
    }

    /// Takes every entry of `other` that's newer than this one's, returning the keys that changed value.
    pub fn merge(&mut self, other: &SharedSettings) -> Vec<String> {
        let mut changed = Vec::new();
        for (key, theirs) in &other.entries {
            let before = self.get(key).cloned();
            if self.set(key, theirs.value.clone(), theirs.stamp.clone()) && before.as_ref() != Some(&theirs.value) {
                changed.push(key.clone());
            }
        }
        changed
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("shared settings always serialize")
    }

    /// Reads a stored copy; one that doesn't parse counts as empty, so a damaged copy is rewritten rather than stuck.
    pub fn from_json(text: &str) -> SharedSettings {
        serde_json::from_str(text.trim()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_newest_change_to_each_setting_wins() {
        let mut a = SharedSettings::default();
        a.set("opacity", json!(80), Stamp::new(2_000, 0, "pc-a"));
        a.set("hotkey", json!("Ctrl+Space"), Stamp::new(1_000, 0, "pc-a"));

        let mut b = SharedSettings::default();
        b.set("opacity", json!(60), Stamp::new(1_500, 0, "pc-b"));
        b.set("hotkey", json!("Alt+Space"), Stamp::new(3_000, 0, "pc-b"));

        assert_eq!(a.merge(&b), ["hotkey"]);
        assert_eq!(a.get("opacity"), Some(&json!(80)));
        assert_eq!(a.get("hotkey"), Some(&json!("Alt+Space")));

        assert_eq!(b.merge(&a), ["opacity"]);
        assert_eq!(a, b, "both sides end up the same");
        assert!(a.merge(&b).is_empty());
    }

    #[test]
    fn an_older_change_never_replaces_a_newer_one() {
        let mut s = SharedSettings::default();
        assert!(s.set("detection", json!(false), Stamp::new(5, 0, "pc")));
        assert!(!s.set("detection", json!(true), Stamp::new(4, 0, "pc")));
        assert_eq!(s.get("detection"), Some(&json!(false)));
        assert_eq!(SharedSettings::from_json(&s.to_json()), s);
        assert_eq!(SharedSettings::from_json("not json"), SharedSettings::default());
    }
}
