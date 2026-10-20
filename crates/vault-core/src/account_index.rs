//! Optional plaintext account metadata for suggestions while the vault is locked.
//! This file never contains passwords, notes, or full website paths. Sharing the
//! parser keeps the desktop and native host's validation rules identical.
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

pub const MAX_INDEX_BYTES: u64 = 512 * 1024;
pub const MAX_INDEX_ENTRIES: usize = 512;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserSettings {
    pub show_accounts_when_locked: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    pub id: String,
    pub title: String,
    pub username: String,
    pub origin: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub settings: BrowserSettings,
    pub entries: Vec<IndexEntry>,
}

/// Read only the explicit opt-in metadata format, bounded before JSON parsing.
/// Missing, malformed, disabled-with-data, or oversized files fail closed.
pub fn read_index(path: &Path) -> Option<Index> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return None;
    }
    let index: Index = serde_json::from_slice(&bytes).ok()?;
    if index.entries.len() > MAX_INDEX_ENTRIES
        || (!index.settings.show_accounts_when_locked && !index.entries.is_empty())
        || index.entries.iter().any(|entry| {
            entry.id.is_empty()
                || entry.id.len() > 128
                || entry.title.len() > 1024
                || entry.username.len() > 2048
                || crate::validate_origin(&entry.origin).is_err()
        })
    {
        return None;
    }
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn synthetic_entry() -> IndexEntry {
        IndexEntry {
            id: "synthetic-account".into(),
            title: "Synthetic account".into(),
            username: "synthetic@example.test".into(),
            origin: "https://example.test".into(),
        }
    }
    fn write(path: &Path, index: &Index) {
        fs::write(path, serde_json::to_vec(index).unwrap()).unwrap();
    }

    #[test]
    fn opt_in_metadata_roundtrips_and_default_is_disabled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("browser-index.json");
        assert!(read_index(&path).is_none());
        write(&path, &Index::default());
        assert!(
            !read_index(&path)
                .unwrap()
                .settings
                .show_accounts_when_locked
        );
        let index = Index {
            settings: BrowserSettings {
                show_accounts_when_locked: true,
            },
            entries: vec![synthetic_entry()],
        };
        write(&path, &index);
        let loaded = read_index(&path).unwrap();
        assert_eq!(loaded.entries[0].username, "synthetic@example.test");
        assert_eq!(loaded.entries[0].origin, "https://example.test");
    }

    #[test]
    fn malformed_unknown_secret_fields_and_disabled_data_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("browser-index.json");
        for text in [
            "not-json",
            r#"{"settings":{"showAccountsWhenLocked":true},"entries":[],"password":"synthetic-secret"}"#,
            r#"{"settings":{"showAccountsWhenLocked":true},"entries":[{"id":"x","title":"x","username":"x","origin":"https://example.test","password":"synthetic-secret"}]}"#,
            r#"{"settings":{"showAccountsWhenLocked":false},"entries":[{"id":"x","title":"x","username":"x","origin":"https://example.test"}]}"#,
        ] {
            fs::write(&path, text).unwrap();
            assert!(read_index(&path).is_none());
        }
    }

    #[test]
    fn byte_entry_field_and_canonical_origin_limits_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("browser-index.json");
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_INDEX_BYTES + 1)
            .unwrap();
        assert!(read_index(&path).is_none());
        let mut index = Index {
            settings: BrowserSettings {
                show_accounts_when_locked: true,
            },
            entries: vec![synthetic_entry(); MAX_INDEX_ENTRIES + 1],
        };
        write(&path, &index);
        assert!(read_index(&path).is_none());
        index.entries = vec![synthetic_entry()];
        for origin in [
            "http://example.test",
            "https://example.test/login",
            "https://example.test/",
            "https://user@example.test",
        ] {
            index.entries[0].origin = origin.into();
            write(&path, &index);
            assert!(read_index(&path).is_none());
        }
        index.entries[0] = synthetic_entry();
        index.entries[0].id.clear();
        write(&path, &index);
        assert!(read_index(&path).is_none());
        index.entries[0] = synthetic_entry();
        index.entries[0].username = "x".repeat(2049);
        write(&path, &index);
        assert!(read_index(&path).is_none());
        index.entries[0] = synthetic_entry();
        index.entries[0].title = "x".repeat(1025);
        write(&path, &index);
        assert!(read_index(&path).is_none());
        index.entries[0] = synthetic_entry();
        index.entries[0].id = "x".repeat(129);
        write(&path, &index);
        assert!(read_index(&path).is_none());
    }
}
