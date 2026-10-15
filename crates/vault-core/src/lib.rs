//! Local vault operations. No network, clipboard, or browser access.
use fs2::FileExt;
mod matching;
mod preflight;
use keepass::{
    config::{CompressionConfig, DatabaseConfig, DatabaseVersion, KdfConfig},
    db::{fields, EntryId},
    Database, DatabaseKey,
};
pub use matching::validate_origin;
use rand::{rngs::OsRng, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use thiserror::Error;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

const MAX_VAULT_BYTES: u64 = 32 * 1024 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("Unlock your vault first.")]
    Locked,
    #[error("A vault already exists. Unlock it instead.")]
    Exists,
    #[error("No vault exists yet. Create one first.")]
    Missing,
    #[error("Couldn't unlock: incorrect password, unsupported format, or damaged vault.")]
    Unlock,
    #[error("Couldn't write the encrypted vault. Your previous file was preserved.")]
    Save,
    #[error("Another Fakkarni instance is using this vault.")]
    InUse,
    #[error("The vault changed outside Fakkarni. Lock and reopen it before saving.")]
    Changed,
    #[error("That entry no longer exists.")]
    NotFound,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("Couldn't access local vault storage.")]
    Storage,
}
pub type Result<T> = std::result::Result<T, VaultError>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub exists: bool,
    pub unlocked: bool,
    pub path: String,
    pub idle_minutes: u32,
}

#[derive(Clone, Serialize)]
pub struct EntrySummary {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
}

#[derive(Deserialize, Serialize, Zeroize, ZeroizeOnDrop)]
pub struct EntryInput {
    pub id: Option<String>,
    pub title: String,
    pub username: String,
    pub url: String,
    pub password: String,
    pub notes: String,
}

struct Session {
    database: Database,
    key: DatabaseKey,
    fingerprint: [u8; 32],
    last_activity: Instant,
    _file_lock: File,
}

pub struct Vault {
    path: PathBuf,
    session: Option<Session>,
}

impl Vault {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            session: None,
        }
    }

    pub fn status(&mut self) -> Status {
        self.expire();
        Status {
            exists: self.path.exists(),
            unlocked: self.session.is_some(),
            path: self.path.display().to_string(),
            idle_minutes: 5,
        }
    }

    pub fn lock(&mut self) {
        self.session = None;
    }

    pub fn expire(&mut self) {
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.last_activity.elapsed() >= IDLE_TIMEOUT)
        {
            self.lock();
        }
    }

    pub fn touch(&mut self) -> Result<()> {
        self.active()?;
        Ok(())
    }

    fn active(&mut self) -> Result<&mut Session> {
        self.expire();
        let session = self.session.as_mut().ok_or(VaultError::Locked)?;
        session.last_activity = Instant::now();
        Ok(session)
    }

    fn acquire_lock(&self) -> Result<File> {
        let parent = self.path.parent().ok_or(VaultError::Storage)?;
        fs::create_dir_all(parent).map_err(|_| VaultError::Storage)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("kdbx.lock"))
            .map_err(|_| VaultError::Storage)?;
        FileExt::try_lock_exclusive(&file).map_err(|_| VaultError::InUse)?;
        Ok(file)
    }

    pub fn create(&mut self, password: &str) -> Result<()> {
        self.lock();
        if password.chars().count() < 12 || password.len() > 1024 {
            return Err(VaultError::Invalid(
                "Use a master password of at least 12 characters (maximum 1024 bytes).",
            ));
        }
        let file_lock = self.acquire_lock()?;
        if self.path.exists() {
            return Err(VaultError::Exists);
        }
        let mut config = DatabaseConfig::default();
        config.version = DatabaseVersion::KDB4(1);
        // No decompression of attacker-controlled data in the initial reader.
        config.compression_config = CompressionConfig::None;
        config.kdf_config = KdfConfig::Argon2id {
            iterations: 3,
            memory: 64 * 1024 * 1024,
            parallelism: 1,
            version: argon2::Version::Version13,
        };
        let mut database = Database::with_config(config);
        database.root_mut().name = "Fakkarni".into();
        let key = DatabaseKey::new().with_password(password);
        let fingerprint = persist(&self.path, &database, &key, None)?;
        self.session = Some(Session {
            database,
            key,
            fingerprint,
            last_activity: Instant::now(),
            _file_lock: file_lock,
        });
        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<()> {
        self.lock();
        if password.len() > 1024 {
            return Err(VaultError::Unlock);
        }
        let file_lock = self.acquire_lock()?;
        if !self.path.exists() {
            return Err(VaultError::Missing);
        }
        let bytes = read_vault(&self.path)?;
        preflight::validate(&bytes)?;
        let key = DatabaseKey::new().with_password(password);
        let database = Database::parse(&bytes, key.clone()).map_err(|_| VaultError::Unlock)?;
        // This milestone supports only vaults created by Fakkarni. General import is deferred.
        if database.config.version != DatabaseVersion::KDB4(1) {
            return Err(VaultError::Unlock);
        }
        let fingerprint = Sha256::digest(&bytes).into();
        self.session = Some(Session {
            database,
            key,
            fingerprint,
            last_activity: Instant::now(),
            _file_lock: file_lock,
        });
        Ok(())
    }

    pub fn list(&mut self) -> Result<Vec<EntrySummary>> {
        let session = self.active()?;
        let mut entries: Vec<_> = session
            .database
            .iter_all_entries()
            .map(|e| EntrySummary {
                id: e.id().uuid().to_string(),
                title: e.get(fields::TITLE).unwrap_or_default().into(),
                username: e.get(fields::USERNAME).unwrap_or_default().into(),
                url: e.get(fields::URL).unwrap_or_default().into(),
            })
            .collect();
        entries.sort_by_key(|e| e.title.to_lowercase());
        Ok(entries)
    }

    /// Explicit edit operation, not used for browsing or search.
    pub fn edit(&mut self, id: &str) -> Result<EntryInput> {
        let session = self.active()?;
        let entry = session
            .database
            .entry(parse_id(id)?)
            .ok_or(VaultError::NotFound)?;
        Ok(EntryInput {
            id: Some(id.into()),
            title: entry.get(fields::TITLE).unwrap_or_default().into(),
            username: entry.get(fields::USERNAME).unwrap_or_default().into(),
            url: entry.get(fields::URL).unwrap_or_default().into(),
            password: entry.get(fields::PASSWORD).unwrap_or_default().into(),
            notes: entry.get(fields::NOTES).unwrap_or_default().into(),
        })
    }

    pub fn reveal(&mut self, id: &str) -> Result<String> {
        let session = self.active()?;
        let entry = session
            .database
            .entry(parse_id(id)?)
            .ok_or(VaultError::NotFound)?;
        Ok(entry.get(fields::PASSWORD).unwrap_or_default().into())
    }

    pub fn save_entry(&mut self, input: &EntryInput) -> Result<String> {
        validate(input)?;
        let path = self.path.clone();
        let session = self.active()?;
        // Commit the in-memory change only after the encrypted file is safely replaced.
        let mut next = session.database.clone();
        let entry_id;
        {
            let mut root = next.root_mut();
            let mut entry = if let Some(id) = &input.id {
                root.entry_mut(parse_id(id)?).ok_or(VaultError::NotFound)?
            } else {
                root.add_entry()
            };
            entry_id = entry.id();
            entry.set_protected(fields::TITLE, input.title.trim());
            entry.set_protected(fields::USERNAME, input.username.as_str());
            entry.set_protected(fields::URL, input.url.trim());
            entry.set_protected(fields::PASSWORD, input.password.as_str());
            entry.set_protected(fields::NOTES, input.notes.as_str());
        }
        let fingerprint = persist(&path, &next, &session.key, Some(session.fingerprint))?;
        session.database = next;
        session.fingerprint = fingerprint;
        Ok(entry_id.uuid().to_string())
    }
}

fn validate(input: &EntryInput) -> Result<()> {
    if input.title.trim().is_empty() {
        return Err(VaultError::Invalid("Give this login a name."));
    }
    if input.title.len() > 256
        || input.username.len() > 4096
        || input.password.len() > 4096
        || input.url.len() > 2048
        || input.notes.len() > 16384
    {
        return Err(VaultError::Invalid(
            "One or more fields exceed their size limit.",
        ));
    }
    if !input.url.trim().is_empty() {
        let url = url::Url::parse(input.url.trim()).map_err(|_| {
            VaultError::Invalid("Enter a full HTTPS website address, or leave it empty for an app.")
        })?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(VaultError::Invalid(
                "Use an HTTPS website address without embedded credentials.",
            ));
        }
    }
    Ok(())
}

fn parse_id(id: &str) -> Result<EntryId> {
    Uuid::parse_str(id)
        .map(EntryId::from_uuid)
        .map_err(|_| VaultError::NotFound)
}

fn read_vault(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = File::open(path).map_err(|_| VaultError::Storage)?;
    let mut bytes = Vec::new();
    file.take(MAX_VAULT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| VaultError::Storage)?;
    if bytes.len() as u64 > MAX_VAULT_BYTES {
        return Err(VaultError::Invalid(
            "Vault exceeds the current 32 MiB limit.",
        ));
    }
    Ok(bytes)
}

fn persist(
    path: &Path,
    db: &Database,
    key: &DatabaseKey,
    expected: Option<[u8; 32]>,
) -> Result<[u8; 32]> {
    let mut encrypted = Vec::new();
    db.save(&mut encrypted, key.clone())
        .map_err(|_| VaultError::Save)?;
    if encrypted.len() as u64 > MAX_VAULT_BYTES {
        return Err(VaultError::Save);
    }
    let mut temporary = NamedTempFile::new_in(path.parent().ok_or(VaultError::Storage)?)
        .map_err(|_| VaultError::Save)?;
    temporary
        .write_all(&encrypted)
        .map_err(|_| VaultError::Save)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| VaultError::Save)?;
    if let Some(expected) = expected {
        let actual: [u8; 32] = Sha256::digest(read_vault(path)?).into();
        if actual != expected {
            return Err(VaultError::Changed);
        }
        temporary.persist(path).map_err(|_| VaultError::Save)?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|_| VaultError::Exists)?;
    }
    Ok(Sha256::digest(encrypted).into())
}

pub fn generate_password(length: usize) -> Result<String> {
    if !(16..=128).contains(&length) {
        return Err(VaultError::Invalid(
            "Choose a password length between 16 and 128.",
        ));
    }
    const ALPHABET: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-_=+[]{}:,.?";
    let mut rng = OsRng;
    Ok((0..length)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiration_revokes_access() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::new(dir.path().join("test.kdbx"));
        vault.create("synthetic master password").unwrap();
        vault.session.as_mut().unwrap().last_activity = Instant::now() - IDLE_TIMEOUT;
        assert!(matches!(
            vault.reveal(&Uuid::new_v4().to_string()),
            Err(VaultError::Locked)
        ));
        assert!(!vault.status().unlocked);
    }
}
