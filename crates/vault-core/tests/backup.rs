use fakkarni_vault::{EntryInput, Vault, VaultError};
use std::{fs, path::Path};

const OLD_MASTER: &str = "Synthetic original vault master";
const BACKUP_MASTER: &str = "Synthetic backup vault master";
fn create(path: &Path, master: &str, label: &str) -> (Vault, String) {
    let mut vault = Vault::new(path.into());
    vault.create(master).unwrap();
    let id = vault
        .save_entry(&EntryInput {
            id: None,
            title: label.into(),
            username: format!("{label}@example.test"),
            password: format!("Secret separate from username {label}"),
            url: "https://example.test/login".into(),
            notes: format!("Private note {label}"),
        })
        .unwrap();
    (vault, id)
}
#[test]
fn encrypted_export_is_exact_requires_unlock_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let (mut vault, id) = create(&path, OLD_MASTER, "Original");
    let backup = dir.path().join("backup.kdbx");
    vault.export_backup(&backup).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), fs::read(&path).unwrap());
    let mut independently_opened = Vault::new(backup.clone());
    independently_opened.unlock(OLD_MASTER).unwrap();
    assert_eq!(
        independently_opened.reveal(&id).unwrap(),
        "Secret separate from username Original"
    );
    assert!(vault.export_backup(&backup).is_err());
    assert!(vault.export_backup(&path).is_err());
    assert!(vault
        .export_backup(&path.with_extension("kdbx.lock"))
        .is_err());
    fs::write(&path, b"external replacement").unwrap();
    assert!(matches!(
        vault.export_backup(&dir.path().join("changed.kdbx")),
        Err(VaultError::Changed)
    ));
    assert!(!dir.path().join("changed.kdbx").exists());
    vault.lock();
    assert!(matches!(
        vault.export_backup(&dir.path().join("locked.kdbx")),
        Err(VaultError::Locked)
    ));
}
#[test]
fn successful_restore_preserves_encrypted_safety_copy_and_requires_reunlock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let backup = dir.path().join("backup.kdbx");
    let (mut vault, old_id) = create(&path, OLD_MASTER, "Original");
    let (mut source, new_id) = create(&backup, BACKUP_MASTER, "Backup");
    source.lock();
    let old_bytes = fs::read(&path).unwrap();
    let backup_bytes = fs::read(&backup).unwrap();
    let prepared = vault.prepare_restore(&backup, BACKUP_MASTER).unwrap();
    assert_eq!(prepared.entries(), 1);
    assert!(prepared.replaces_existing());
    assert!(!prepared.identity().is_empty());
    assert_eq!(
        vault.reveal(&old_id).unwrap(),
        "Secret separate from username Original"
    );
    assert_eq!(fs::read(&path).unwrap(), old_bytes);
    let report = vault
        .commit_restore(prepared, &dir.path().join("recovery"))
        .unwrap();
    assert_eq!(report.entries, 1);
    assert!(!vault.status().unlocked);
    assert_eq!(fs::read(&path).unwrap(), backup_bytes);
    let recovery = report.recovery_path.unwrap();
    assert_eq!(fs::read(&recovery).unwrap(), old_bytes);
    let mut prior = Vault::new(recovery);
    prior.unlock(OLD_MASTER).unwrap();
    assert_eq!(
        prior.reveal(&old_id).unwrap(),
        "Secret separate from username Original"
    );
    assert!(vault.unlock(OLD_MASTER).is_err());
    vault.unlock(BACKUP_MASTER).unwrap();
    assert_eq!(
        vault.reveal(&new_id).unwrap(),
        "Secret separate from username Backup"
    );
}
#[test]
fn missing_target_restore_and_locked_existing_target_use_cooperative_lock() {
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("backup.kdbx");
    let (mut source, id) = create(&backup, BACKUP_MASTER, "Backup");
    source.lock();
    let mut vault = Vault::new(dir.path().join("new.kdbx"));
    let prepared = vault.prepare_restore(&backup, BACKUP_MASTER).unwrap();
    assert!(!prepared.replaces_existing());
    assert!(vault
        .commit_restore(prepared, &dir.path().join("unused-recovery"))
        .unwrap()
        .recovery_path
        .is_none());
    assert!(!dir.path().join("unused-recovery").exists());
    vault.unlock(BACKUP_MASTER).unwrap();
    assert!(vault.reveal(&id).is_ok());
    vault.lock();
    let prepared = vault.prepare_restore(&backup, BACKUP_MASTER).unwrap();
    let mut second = Vault::new(dir.path().join("new.kdbx"));
    second.unlock(BACKUP_MASTER).unwrap();
    assert!(matches!(
        vault.commit_restore(prepared, &dir.path().join("recovery")),
        Err(VaultError::InUse)
    ));
}
#[test]
fn invalid_candidates_are_rejected_before_any_target_or_session_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let backup = dir.path().join("backup.kdbx");
    let (mut vault, id) = create(&path, OLD_MASTER, "Original");
    let (mut source, _) = create(&backup, BACKUP_MASTER, "Backup");
    source.lock();
    let original = fs::read(&path).unwrap();
    assert!(vault
        .prepare_restore(&backup, "wrong synthetic master")
        .is_err());
    assert!(vault.prepare_restore(&path, OLD_MASTER).is_err());
    assert!(vault
        .prepare_restore(&dir.path().join(".").join("vault.kdbx"), OLD_MASTER)
        .is_err());
    #[cfg(windows)]
    assert!(vault
        .prepare_restore(&dir.path().join("VAULT.KDBX"), OLD_MASTER)
        .is_err());
    assert!(vault
        .prepare_restore(&dir.path().join("missing.kdbx"), BACKUP_MASTER)
        .is_err());
    let corrupt = dir.path().join("corrupt.kdbx");
    fs::write(&corrupt, b"damaged synthetic backup").unwrap();
    assert!(vault.prepare_restore(&corrupt, BACKUP_MASTER).is_err());
    let huge = dir.path().join("huge.kdbx");
    fs::File::create(&huge)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    assert!(vault.prepare_restore(&huge, BACKUP_MASTER).is_err());
    let mut excessive = fs::read(&backup).unwrap();
    let memory = (64_u64 * 1024 * 1024).to_le_bytes();
    let offset = excessive
        .windows(8)
        .position(|part| part == memory)
        .unwrap();
    excessive[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    let excessive_path = dir.path().join("excessive-kdf.kdbx");
    fs::write(&excessive_path, excessive).unwrap();
    assert!(vault
        .prepare_restore(&excessive_path, BACKUP_MASTER)
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        vault.reveal(&id).unwrap(),
        "Secret separate from username Original"
    );
}

#[cfg(windows)]
#[test]
fn failed_atomic_replacement_keeps_old_ciphertext_and_session_after_safety_copy() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let backup = dir.path().join("backup.kdbx");
    let (mut vault, id) = create(&path, OLD_MASTER, "Original");
    let (mut source, _) = create(&backup, BACKUP_MASTER, "Backup");
    source.lock();
    let original = fs::read(&path).unwrap();
    let prepared = vault.prepare_restore(&backup, BACKUP_MASTER).unwrap();
    // This synthetic reader permits read access but denies FILE_SHARE_DELETE,
    // forcing the final Windows atomic rename to fail after recovery succeeds.
    let replacement_blocker = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let recovery = dir.path().join("recovery");
    assert!(vault.commit_restore(prepared, &recovery).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(vault.status().unlocked);
    assert_eq!(
        vault.reveal(&id).unwrap(),
        "Secret separate from username Original"
    );
    let backups: Vec<_> = fs::read_dir(&recovery)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), original);
    drop(replacement_blocker);
    vault.lock();
    vault.unlock(OLD_MASTER).unwrap();
    assert!(vault.reveal(&id).is_ok());
}
#[test]
fn changed_source_target_and_recovery_failure_preserve_existing_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let backup = dir.path().join("backup.kdbx");
    let (mut vault, id) = create(&path, OLD_MASTER, "Original");
    let (mut source, _) = create(&backup, BACKUP_MASTER, "Backup");
    source.lock();
    let original = fs::read(&path).unwrap();
    let source_bytes = fs::read(&backup).unwrap();
    let prepared = vault.prepare_restore(&backup, BACKUP_MASTER).unwrap();
    fs::write(&backup, b"changed source").unwrap();
    assert!(matches!(
        vault.commit_restore(prepared, &dir.path().join("recovery")),
        Err(VaultError::Changed)
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::write(&backup, &source_bytes).unwrap();
    let prepared = vault.prepare_restore(&backup, BACKUP_MASTER).unwrap();
    let bad_recovery = dir.path().join("recovery-file");
    fs::write(&bad_recovery, b"not a directory").unwrap();
    assert!(vault.commit_restore(prepared, &bad_recovery).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        vault.reveal(&id).unwrap(),
        "Secret separate from username Original"
    );
    let prepared = vault.prepare_restore(&backup, BACKUP_MASTER).unwrap();
    fs::write(&path, b"changed target").unwrap();
    assert!(matches!(
        vault.commit_restore(prepared, &dir.path().join("recovery")),
        Err(VaultError::Changed)
    ));
    assert_eq!(fs::read(&path).unwrap(), b"changed target");
    assert_eq!(
        vault.reveal(&id).unwrap(),
        "Secret separate from username Original"
    );
}
