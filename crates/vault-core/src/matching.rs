//! Browser-origin matching never exposes credentials or extends an unlock session.
use crate::{fields, EntrySummary, Result, Vault, VaultError};
use url::Url;

/// Accept only the canonical ASCII HTTPS origin supplied by a browser.
/// An origin has no trailing slash, path, query, fragment, or credentials.
pub fn validate_origin(origin: &str) -> Result<String> {
    if origin.len() > 2048 {
        return Err(VaultError::Invalid("Invalid browser origin."));
    }
    let parsed = Url::parse(origin).map_err(|_| VaultError::Invalid("Invalid browser origin."))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.origin().ascii_serialization() != origin
    {
        return Err(VaultError::Invalid("Use a canonical HTTPS browser origin."));
    }
    Ok(origin.to_owned())
}

fn stored_origin_matches(stored: &str, origin: &str) -> bool {
    let Ok(url) = Url::parse(stored) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.origin().ascii_serialization() == origin
}

impl Vault {
    /// Metadata for one exact HTTPS origin; requests do not renew idle timeout.
    pub fn matches(&mut self, origin: &str) -> Result<Vec<EntrySummary>> {
        self.expire();
        let session = self.session.as_ref().ok_or(VaultError::Locked)?;
        let origin = validate_origin(origin)?;
        let mut entries: Vec<_> = session
            .database
            .iter_all_entries()
            .filter(|entry| {
                stored_origin_matches(entry.get(fields::URL).unwrap_or_default(), &origin)
            })
            .map(|entry| EntrySummary {
                id: entry.id().uuid().to_string(),
                title: entry.get(fields::TITLE).unwrap_or_default().into(),
                username: entry.get(fields::USERNAME).unwrap_or_default().into(),
                url: entry.get(fields::URL).unwrap_or_default().into(),
            })
            .collect();
        entries.sort_by_key(|entry| entry.title.to_lowercase());
        entries.truncate(50);
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Database, DatabaseKey, Session, IDLE_TIMEOUT};
    use std::time::Instant;

    #[test]
    fn accepts_only_canonical_https_origins() {
        for origin in [
            "https://example.test",
            "https://example.test:8443",
            "https://xn--bcher-kva.example",
            "https://[::1]",
        ] {
            assert_eq!(validate_origin(origin).unwrap(), origin);
        }
        for origin in [
            "http://example.test",
            "file:///example.test",
            "null",
            "https://EXAMPLE.test",
            "https://example.test/",
            "https://example.test/login",
            "https://example.test?x=1",
            "https://example.test#fragment",
            "https://user@example.test",
            "https://:password@example.test",
            "https://example.test:443",
            "https://bücher.example",
            " https://example.test",
            "https://example.test\n",
            "https://example.test\\attacker.test",
        ] {
            assert!(validate_origin(origin).is_err(), "accepted {origin:?}");
        }
    }

    #[test]
    fn exact_origin_blocks_phishing_subdomains_and_port_changes() {
        assert!(stored_origin_matches(
            "https://EXAMPLE.test:443/login?q=1",
            "https://example.test"
        ));
        assert!(stored_origin_matches(
            "https://bücher.example/login",
            "https://xn--bcher-kva.example"
        ));
        assert!(stored_origin_matches(
            "https://example.test:8443/login",
            "https://example.test:8443"
        ));
        for stored in [
            "https://example.test.attacker.test",
            "https://attackerexample.test",
            "https://login.example.test",
            "https://example.test:8443",
            "http://example.test",
            "https://example.test@attacker.test",
            "https://user@example.test",
            "https://examp1e.test",
            "",
        ] {
            assert!(
                !stored_origin_matches(stored, "https://example.test"),
                "matched {stored:?}"
            );
        }
    }

    #[test]
    fn query_is_bounded_sorted_and_does_not_refresh_or_bypass_lock() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::new(dir.path().join("synthetic.kdbx"));
        assert!(matches!(
            vault.matches("https://example.test"),
            Err(VaultError::Locked)
        ));
        let mut database = Database::new();
        for index in (0..55).rev() {
            let mut root = database.root_mut();
            let mut entry = root.add_entry();
            entry.set_protected(fields::TITLE, format!("Login {index:02}"));
            entry.set_protected(fields::USERNAME, "synthetic-user");
            entry.set_protected(fields::PASSWORD, "must-not-be-returned");
            entry.set_protected(fields::NOTES, "must-not-be-returned-either");
            entry.set_protected(fields::URL, "https://example.test/login");
        }
        {
            let mut root = database.root_mut();
            let mut other = root.add_entry();
            other.set_protected(fields::TITLE, "Other site");
            other.set_protected(fields::URL, "https://other.test");
        }
        let last_activity = Instant::now();
        vault.session = Some(Session {
            database,
            key: DatabaseKey::new(),
            fingerprint: [0; 32],
            last_activity,
            _file_lock: vault.acquire_lock().unwrap(),
        });
        let matches = vault.matches("https://example.test").unwrap();
        assert_eq!(matches.len(), 50);
        assert_eq!(matches[0].title, "Login 00");
        assert_eq!(matches[49].title, "Login 49");
        assert_eq!(vault.session.as_ref().unwrap().last_activity, last_activity);
        let serialized = serde_json::to_string(&matches).unwrap();
        assert!(!serialized.contains("must-not-be-returned"));
        assert!(vault.matches("https://absent.test").unwrap().is_empty());
        vault.session.as_mut().unwrap().last_activity = Instant::now() - IDLE_TIMEOUT;
        assert!(matches!(
            vault.matches("https://example.test"),
            Err(VaultError::Locked)
        ));
        assert!(vault.session.is_none());
    }
}
