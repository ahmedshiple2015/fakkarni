//! A portable encrypted envelope, separate from both the KDBX and recovery key.
use crate::{Database, Result, Vault, VaultError};
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use keepass::db::{CustomDataItem, CustomDataValue};
use rand::{rngs::OsRng, RngCore};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"FKREC001";
const HEADER: usize = 8 + 16 + 16 + 12;
const MAX: usize = HEADER + 1024 + 16;
const GENERATION: &str = "org.fakkarni.password-generation";
const INVALID: VaultError = VaultError::Invalid("The recovery key or file does not match.");

pub(crate) fn generation(db: &Database) -> Result<[u8; 16]> {
    match db.meta.custom_data.get(GENERATION) {
        None => Ok([0; 16]),
        Some(item) => match &item.value {
            Some(CustomDataValue::String(value)) => uuid::Uuid::parse_str(value)
                .map(|id| *id.as_bytes())
                .map_err(|_| INVALID),
            _ => Err(INVALID),
        },
    }
}

pub(crate) fn rotate_generation(db: &mut Database) {
    db.meta.custom_data.insert(
        GENERATION.into(),
        CustomDataItem {
            value: Some(CustomDataValue::String(uuid::Uuid::new_v4().to_string())),
            last_modification_time: None,
        },
    );
}

pub struct RecoverySecret {
    pub password: Zeroizing<String>,
    pub identity: String,
    pub generation: Option<[u8; 16]>,
}

fn random(bytes: &mut [u8]) -> Result<()> {
    OsRng.try_fill_bytes(bytes).map_err(|_| VaultError::Storage)
}

fn decode_key(key: &str) -> Result<Zeroizing<[u8; 32]>> {
    let key = key.trim();
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(INVALID);
    }
    let mut bytes = Zeroizing::new([0; 32]);
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&key[i * 2..i * 2 + 2], 16).map_err(|_| INVALID)?;
    }
    Ok(bytes)
}

pub fn open_envelope(path: &Path, key: &str) -> Result<RecoverySecret> {
    let file = File::open(path).map_err(|_| VaultError::Storage)?;
    if !file.metadata().map_err(|_| VaultError::Storage)?.is_file() {
        return Err(INVALID);
    }
    let mut bytes = Vec::new();
    file.take((MAX + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| VaultError::Storage)?;
    if !(HEADER + 17..=MAX).contains(&bytes.len()) || &bytes[..8] != MAGIC {
        return Err(INVALID);
    }
    let key = decode_key(key)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| INVALID)?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&bytes[40..HEADER]),
                Payload {
                    msg: &bytes[HEADER..],
                    aad: &bytes[..HEADER],
                },
            )
            .map_err(|_| INVALID)?,
    );
    let password = Zeroizing::new(
        std::str::from_utf8(&plaintext)
            .map_err(|_| INVALID)?
            .to_owned(),
    );
    Ok(RecoverySecret {
        password,
        identity: uuid::Uuid::from_slice(&bytes[8..24])
            .map_err(|_| INVALID)?
            .to_string(),
        generation: Some(bytes[24..40].try_into().map_err(|_| INVALID)?),
    })
}

impl Vault {
    pub fn export_recovery_envelope(
        &mut self,
        password: &str,
        path: &Path,
        allowed: impl Fn() -> bool,
    ) -> Result<Zeroizing<String>> {
        self.verify_password(password)?;
        let db = &self.session.as_ref().ok_or(VaultError::Locked)?.database;
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        random(key.as_mut())?;
        random(&mut nonce)?;
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(db.root().id().uuid().as_bytes());
        bytes.extend_from_slice(&generation(db)?);
        bytes.extend_from_slice(&nonce);
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| INVALID)?;
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: password.as_bytes(),
                    aad: &bytes,
                },
            )
            .map_err(|_| VaultError::Save)?;
        bytes.extend(encrypted);
        let mut staged = NamedTempFile::new_in(path.parent().ok_or(VaultError::Storage)?)
            .map_err(|_| VaultError::Save)?;
        staged
            .write_all(&bytes)
            .and_then(|_| staged.as_file().sync_all())
            .map_err(|_| VaultError::Save)?;
        if !allowed() {
            return Err(VaultError::Changed);
        }
        staged.persist_noclobber(path).map_err(|_| {
            VaultError::Invalid("Could not save the recovery file. Choose a new filename.")
        })?;
        use std::fmt::Write as _;
        let mut text = Zeroizing::new(String::with_capacity(64));
        for byte in key.iter() {
            write!(&mut *text, "{byte:02x}").expect("writing to String");
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntryInput;
    use std::{cell::Cell, fs};
    const OLD: &str = "synthetic original master phrase";
    const NEW: &str = "synthetic replacement master phrase";

    #[test]
    fn portable_recovery_preserves_entries_and_refuses_wrong_files_or_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = Vault::new(dir.path().join("source.kdbx"));
        source.create(OLD).unwrap();
        let id = source
            .save_entry(&EntryInput {
                id: None,
                title: "Synthetic account".into(),
                username: "user@example.test".into(),
                url: "https://example.test".into(),
                password: "site-secret".into(),
                notes: "Keep this".into(),
            })
            .unwrap();
        let envelope = dir.path().join("recovery.fkrec");
        let key = source
            .export_recovery_envelope(OLD, &envelope, || true)
            .unwrap();
        assert!(source
            .export_recovery_envelope(OLD, &envelope, || true)
            .is_err());
        let bytes = fs::read(&envelope).unwrap();
        assert!(!bytes
            .windows(OLD.len())
            .any(|window| window == OLD.as_bytes()));
        assert!(open_envelope(&envelope, &"00".repeat(32)).is_err());
        for offset in [0, 8, 24, 40, bytes.len() - 1] {
            let mut corrupt = bytes.clone();
            corrupt[offset] ^= 1;
            fs::write(dir.path().join("tampered.fkrec"), corrupt).unwrap();
            assert!(open_envelope(&dir.path().join("tampered.fkrec"), &key).is_err());
        }
        fs::write(dir.path().join("tampered.fkrec"), &bytes[..HEADER]).unwrap();
        assert!(open_envelope(&dir.path().join("tampered.fkrec"), &key).is_err());
        let secret = open_envelope(&envelope, &key).unwrap();
        let backup = dir.path().join("backup.kdbx");
        source.export_backup(&backup).unwrap();
        let other = dir.path().join("other.kdbx");
        Vault::new(other.clone()).create(OLD).unwrap();
        let mut destination = Vault::new(dir.path().join("destination.kdbx"));
        assert!(destination
            .recover_into_empty(&other, &secret, NEW, || true)
            .is_err());
        let allowed = Cell::new(true);
        assert!(destination
            .recover_into_empty(&backup, &secret, NEW, || allowed.replace(false))
            .is_err());
        assert!(!destination.status().exists);
        destination
            .recover_into_empty(&backup, &secret, NEW, || true)
            .unwrap();
        assert!(destination
            .recover_into_empty(&backup, &secret, NEW, || true)
            .is_err());
        assert!(destination.unlock(OLD).is_err());
        destination.unlock(NEW).unwrap();
        assert_eq!(destination.reveal(&id).unwrap(), "site-secret");
        assert_eq!(destination.edit(&id).unwrap().notes, "Keep this");
    }

    #[test]
    fn locked_recovery_is_one_snapshot_and_old_generation_stays_stale_after_password_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::new(dir.path().join("vault.kdbx"));
        vault.create(OLD).unwrap();
        let envelope = dir.path().join("recovery.fkrec");
        let key = vault
            .export_recovery_envelope(OLD, &envelope, || true)
            .unwrap();
        let mut secret = open_envelope(&envelope, &key).unwrap();
        let backup_dir = dir.path().join("snapshots");
        let stamp = vault.recovery_stamp().unwrap();
        vault.lock();
        let identity = std::mem::replace(&mut secret.identity, uuid::Uuid::new_v4().to_string());
        assert!(vault
            .recover_password(&secret, stamp, NEW, &backup_dir, || true)
            .is_err());
        secret.identity = identity;
        assert!(!vault.status().unlocked);
        assert!(vault
            .recover_password(&secret, [0; 32], NEW, &backup_dir, || true)
            .is_err());
        assert!(vault
            .recover_password(&secret, stamp, NEW, &backup_dir, || false)
            .is_err());
        let allowed = Cell::new(true);
        assert!(vault
            .recover_password(&secret, stamp, NEW, &backup_dir, || allowed.replace(false))
            .is_err());
        assert_eq!(vault.recovery_stamp().unwrap(), stamp);
        let safety = vault
            .recover_password(&secret, stamp, NEW, &backup_dir, || true)
            .unwrap();
        assert!(safety.exists());
        assert!(!vault.status().unlocked);
        assert!(vault.unlock(OLD).is_err());
        vault.unlock(NEW).unwrap();
        vault.change_master_password(NEW, OLD, &backup_dir).unwrap();
        let new_stamp = vault.recovery_stamp().unwrap();
        assert!(vault
            .recover_password(&secret, new_stamp, NEW, &backup_dir, || true)
            .is_err());
        // A freshly verified PIN uses the same rotation path without an offline envelope.
        secret.generation = None;
        vault
            .recover_password(&secret, new_stamp, NEW, &backup_dir, || true)
            .unwrap();
        vault.unlock(NEW).unwrap();
    }
}
