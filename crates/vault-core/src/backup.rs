//! Copies and replaces only encrypted KDBX snapshots. No plaintext export exists.
use crate::{
    preflight, read_vault, Database, DatabaseKey, DatabaseVersion, Result, Vault, VaultError,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

/// Validated restore candidate. Only encrypted bytes and preview metadata remain
/// after verification; decrypted contents and the candidate key are dropped.
pub struct PreparedRestore {
    bytes: Vec<u8>,
    source: PathBuf,
    source_fingerprint: [u8; 32],
    target: PathBuf,
    target_fingerprint: Option<[u8; 32]>,
    identity: String,
    entries: usize,
}
impl PreparedRestore {
    pub fn entries(&self) -> usize {
        self.entries
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn replaces_existing(&self) -> bool {
        self.target_fingerprint.is_some()
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub entries: usize,
    pub recovery_path: Option<PathBuf>,
}

fn fingerprint(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn kdbx_path(path: &Path) -> Result<()> {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("kdbx"))
    {
        return Err(VaultError::Invalid("Choose an encrypted .kdbx file."));
    }
    Ok(())
}
fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(VaultError::Invalid(
            "Choose a regular local file, not a symbolic link.",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(VaultError::Storage),
    }
}
/// Canonicalize the parent even when the final file does not exist. This catches
/// relative paths and directory junction aliases without following final links.
fn resolved(path: &Path) -> Result<PathBuf> {
    reject_symlink(path)?;
    if path.exists() {
        return fs::canonicalize(path).map_err(|_| VaultError::Storage);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().ok_or(VaultError::Storage)?;
    Ok(fs::canonicalize(parent)
        .map_err(|_| VaultError::Storage)?
        .join(name))
}
fn optional_snapshot(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => read_vault(path).map(Some),
        Ok(_) => Err(VaultError::Storage),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(VaultError::Storage),
    }
}
fn temporary_snapshot(path: &Path, bytes: &[u8]) -> Result<NamedTempFile> {
    let mut temporary = NamedTempFile::new_in(path.parent().ok_or(VaultError::Storage)?)
        .map_err(|_| VaultError::Save)?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| VaultError::Save)?;
    Ok(temporary)
}
fn copy_noclobber(path: &Path, bytes: &[u8]) -> Result<()> {
    temporary_snapshot(path, bytes)?
        .persist_noclobber(path)
        .map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                VaultError::Invalid("That backup file already exists. Choose a new filename.")
            } else {
                VaultError::Save
            }
        })?;
    Ok(())
}

impl Vault {
    /// Export the exact authenticated encrypted file, never a decrypted database.
    /// Existing destinations are refused rather than overwritten.
    pub fn export_backup(&mut self, destination: &Path) -> Result<()> {
        self.expire();
        let session = self.session.as_ref().ok_or(VaultError::Locked)?;
        kdbx_path(destination)?;
        let target = resolved(destination)?;
        if target == resolved(&self.path)?
            || target == resolved(&self.path.with_extension("kdbx.lock"))?
        {
            return Err(VaultError::Invalid(
                "Choose a backup location different from the active vault.",
            ));
        }
        if fs::symlink_metadata(&target).is_ok() {
            return Err(VaultError::Invalid(
                "That backup file already exists. Choose a new filename.",
            ));
        }
        let bytes = read_vault(&self.path)?;
        if fingerprint(&bytes) != session.fingerprint {
            return Err(VaultError::Changed);
        }
        // The loaded session already authenticated this exact ciphertext; strict
        // preflight also keeps exported files within our supported fixed profile.
        preflight::validate(&bytes)?;
        copy_noclobber(&target, &bytes)
    }

    /// Validate the whole candidate and its master password before preparing a
    /// replacement. No session, target file, or recovery file is changed here.
    pub fn prepare_restore(&self, source: &Path, password: &str) -> Result<PreparedRestore> {
        if password.len() > 1024 {
            return Err(VaultError::Unlock);
        }
        kdbx_path(source)?;
        let source = resolved(source)?;
        let target = resolved(&self.path)?;
        if source == target {
            return Err(VaultError::Invalid(
                "Choose a backup different from the active vault.",
            ));
        }
        let bytes = read_vault(&source)?;
        preflight::validate(&bytes)?;
        let key = DatabaseKey::new().with_password(password);
        let database = Database::parse(&bytes, key.clone()).map_err(|_| VaultError::Unlock)?;
        if database.config.version != DatabaseVersion::KDB4(1) {
            return Err(VaultError::Unlock);
        }
        let target_fingerprint = optional_snapshot(&target)?.as_deref().map(fingerprint);
        if self
            .session
            .as_ref()
            .is_some_and(|session| Some(session.fingerprint) != target_fingerprint)
        {
            return Err(VaultError::Changed);
        }
        Ok(PreparedRestore {
            source_fingerprint: fingerprint(&bytes),
            bytes,
            source,
            target,
            target_fingerprint,
            identity: database.root().id().uuid().to_string(),
            entries: database.iter_all_entries().count(),
        })
    }

    /// Commit a verified encrypted snapshot. An existing encrypted vault is first
    /// copied to a unique recovery file. Any failure leaves the previous session
    /// and target untouched; successful replacement closes the old session.
    pub fn commit_restore(
        &mut self,
        prepared: PreparedRestore,
        recovery_dir: &Path,
    ) -> Result<RestoreReport> {
        if resolved(&self.path)? != prepared.target {
            return Err(VaultError::Changed);
        }
        // Reuse an unlocked session's cooperative lock; reacquiring it would fail
        // against our own handle on Windows. A locked/missing vault acquires one.
        let _file_lock = if self.session.is_none() {
            Some(self.acquire_lock()?)
        } else {
            None
        };
        let source_now = read_vault(&prepared.source)?;
        if fingerprint(&source_now) != prepared.source_fingerprint {
            return Err(VaultError::Changed);
        }
        let current = optional_snapshot(&prepared.target)?;
        if current.as_deref().map(fingerprint) != prepared.target_fingerprint
            || self
                .session
                .as_ref()
                .is_some_and(|session| Some(session.fingerprint) != prepared.target_fingerprint)
        {
            return Err(VaultError::Changed);
        }
        let temporary = temporary_snapshot(&prepared.target, &prepared.bytes)?;
        let recovery_path = if let Some(bytes) = current.as_ref() {
            fs::create_dir_all(recovery_dir).map_err(|_| VaultError::Save)?;
            let recovery_dir = fs::canonicalize(recovery_dir).map_err(|_| VaultError::Save)?;
            let path =
                recovery_dir.join(format!("fakkarni-before-restore-{}.kdbx", Uuid::new_v4()));
            copy_noclobber(&path, bytes)?;
            Some(path)
        } else {
            None
        };
        // Detect a non-cooperating writer during recovery-copy creation, before
        // the atomic replacement. Never undo another writer's changed file.
        if optional_snapshot(&prepared.target)?
            .as_deref()
            .map(fingerprint)
            != prepared.target_fingerprint
        {
            return Err(VaultError::Changed);
        }
        if prepared.target_fingerprint.is_some() {
            temporary
                .persist(&prepared.target)
                .map_err(|_| VaultError::Save)?;
        } else {
            temporary
                .persist_noclobber(&prepared.target)
                .map_err(|_| VaultError::Changed)?;
        }
        self.lock();
        Ok(RestoreReport {
            entries: prepared.entries,
            recovery_path,
        })
    }
}
