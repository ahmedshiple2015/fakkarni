//! Master-key rotation keeps an exact encrypted recovery snapshot before publication.
use crate::{
    preflight, read_vault, Database, DatabaseKey, Result, Vault, VaultError, MAX_VAULT_BYTES,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

fn reject_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(VaultError::Invalid(
            "Symbolic links cannot be used for master-password changes.",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(VaultError::Storage),
    }
}

fn stage(parent: &Path, bytes: &[u8]) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new_in(parent).map_err(|_| VaultError::Save)?;
    file.write_all(bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| VaultError::Save)?;
    Ok(file)
}

impl Vault {
    /// Reauthenticate the current master password, then atomically publish a
    /// verified vault encrypted with the new one. The old ciphertext is retained
    /// in a unique recovery file. Success locks; failures preserve the session.
    pub fn change_master_password(
        &mut self,
        current_password: &str,
        new_password: &str,
        recovery_dir: &Path,
    ) -> Result<PathBuf> {
        self.expire();
        let session = self.session.as_ref().ok_or(VaultError::Locked)?;
        if new_password.chars().count() < 12 || new_password.len() > 1024 {
            return Err(VaultError::Invalid(
                "Use a master password of at least 12 characters (maximum 1024 bytes).",
            ));
        }
        if current_password == new_password {
            return Err(VaultError::Invalid("Choose a different master password."));
        }
        if current_password.len() > 1024 {
            return Err(VaultError::Unlock);
        }
        let lock_path = self.path.with_extension("kdbx.lock");
        reject_link(&self.path)?;
        reject_link(&lock_path)?;
        let original = read_vault(&self.path)?;
        let fingerprint: [u8; 32] = Sha256::digest(&original).into();
        if fingerprint != session.fingerprint {
            return Err(VaultError::Changed);
        }
        preflight::validate(&original)?;
        let authenticated = Database::parse(
            &original,
            DatabaseKey::new().with_password(current_password),
        )
        .map_err(|_| VaultError::Unlock)?;
        if authenticated.root().id() != session.database.root().id() {
            return Err(VaultError::Changed);
        }

        let new_key = DatabaseKey::new().with_password(new_password);
        let mut encrypted = Vec::new();
        session
            .database
            .save(&mut encrypted, new_key.clone())
            .map_err(|_| VaultError::Save)?;
        if encrypted.len() as u64 > MAX_VAULT_BYTES {
            return Err(VaultError::Save);
        }
        preflight::validate(&encrypted)?;
        let verified = Database::parse(&encrypted, new_key).map_err(|_| VaultError::Save)?;
        if verified.root().id() != session.database.root().id()
            || verified.iter_all_entries().count() != session.database.iter_all_entries().count()
        {
            return Err(VaultError::Save);
        }
        // Stage and flush the complete candidate before creating the recovery copy.
        // The unlocked session keeps its cooperative file lock throughout.
        let temporary = stage(self.path.parent().ok_or(VaultError::Storage)?, &encrypted)?;
        reject_link(recovery_dir)?;
        fs::create_dir_all(recovery_dir).map_err(|_| VaultError::Save)?;
        let recovery_dir = fs::canonicalize(recovery_dir).map_err(|_| VaultError::Save)?;
        let recovery_path = recovery_dir.join(format!(
            "fakkarni-before-password-change-{}.kdbx",
            Uuid::new_v4()
        ));
        stage(&recovery_dir, &original)?
            .persist_noclobber(&recovery_path)
            .map_err(|_| VaultError::Save)?;
        // A noncooperating writer must never be overwritten during backup creation.
        reject_link(&self.path)?;
        reject_link(&lock_path)?;
        let actual: [u8; 32] = Sha256::digest(read_vault(&self.path)?).into();
        if actual != fingerprint {
            return Err(VaultError::Changed);
        }
        temporary
            .persist(&self.path)
            .map_err(|_| VaultError::Save)?;
        self.lock();
        Ok(recovery_path)
    }
}
