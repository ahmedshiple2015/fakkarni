use fakkarni_vault::{EntryInput, Vault, VaultError};
use std::{fs, path::Path};

const OLD: &str = "Synthetic original master password";
const NEW: &str = "Synthetic replacement master password";

fn fixture(path: &Path) -> (Vault, String) {
    let mut vault = Vault::new(path.into());
    vault.create(OLD).unwrap();
    let id = vault
        .save_entry(&EntryInput {
            id: None,
            title: "Synthetic login".into(),
            username: "fixture@example.test".into(),
            url: "https://example.test/login".into(),
            password: "Distinct synthetic login secret".into(),
            notes: "Private note retained exactly\nSecond line".into(),
        })
        .unwrap();
    (vault, id)
}

#[test]
fn rotation_preserves_entries_and_identity_and_keeps_exact_old_encrypted_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let (mut vault, id) = fixture(&path);
    let identity = vault.identity().unwrap();
    let original = fs::read(&path).unwrap();
    let recovery = vault
        .change_master_password(OLD, NEW, &dir.path().join("recovery"))
        .unwrap();
    assert!(!vault.status().unlocked);
    assert_eq!(fs::read(&recovery).unwrap(), original);
    assert_ne!(fs::read(&path).unwrap(), original);
    assert!(vault.unlock(OLD).is_err());
    vault.unlock(NEW).unwrap();
    assert_eq!(vault.identity().unwrap(), identity);
    assert_eq!(vault.list().unwrap().len(), 1);
    let entry = vault.edit(&id).unwrap();
    assert_eq!(entry.id.as_deref(), Some(id.as_str()));
    assert_eq!(entry.title, "Synthetic login");
    assert_eq!(entry.username, "fixture@example.test");
    assert_eq!(entry.url, "https://example.test/login");
    assert_eq!(entry.password, "Distinct synthetic login secret");
    assert_eq!(entry.notes, "Private note retained exactly\nSecond line");
    let mut old = Vault::new(recovery);
    assert!(old.unlock(NEW).is_err());
    old.unlock(OLD).unwrap();
    assert_eq!(old.identity().unwrap(), identity);
    assert_eq!(old.reveal(&id).unwrap(), entry.password);
}

#[test]
fn rejected_changes_preserve_file_and_session_and_have_no_recovery_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let recovery = dir.path().join("recovery");
    let (mut vault, id) = fixture(&path);
    let original = fs::read(&path).unwrap();
    for (current, next) in [
        ("wrong current password", NEW),
        (OLD, "short"),
        (OLD, OLD),
        (OLD, &"a".repeat(1025)),
    ] {
        assert!(vault
            .change_master_password(current, next, &recovery)
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!recovery.exists());
        assert_eq!(
            vault.reveal(&id).unwrap(),
            "Distinct synthetic login secret"
        );
    }
    vault.lock();
    assert!(matches!(
        vault.change_master_password(OLD, NEW, &recovery),
        Err(VaultError::Locked)
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!recovery.exists());
}

#[test]
fn backup_failure_and_external_replacement_preserve_existing_file_and_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let (mut vault, id) = fixture(&path);
    let original = fs::read(&path).unwrap();
    let unusable = dir.path().join("not-a-directory");
    fs::write(&unusable, b"leave this file untouched").unwrap();
    assert!(vault.change_master_password(OLD, NEW, &unusable).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read(&unusable).unwrap(), b"leave this file untouched");
    assert_eq!(
        vault.reveal(&id).unwrap(),
        "Distinct synthetic login secret"
    );
    fs::write(&path, b"noncooperating external replacement").unwrap();
    let recovery = dir.path().join("recovery");
    assert!(matches!(
        vault.change_master_password(OLD, NEW, &recovery),
        Err(VaultError::Changed)
    ));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"noncooperating external replacement"
    );
    assert!(!recovery.exists());
    assert_eq!(
        vault.reveal(&id).unwrap(),
        "Distinct synthetic login secret"
    );
}

#[cfg(windows)]
#[test]
fn atomic_publication_failure_preserves_old_vault_and_recovery_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let (mut vault, id) = fixture(&path);
    let original = fs::read(&path).unwrap();
    // Denying delete-sharing permits reads, but prevents atomic replacement on Windows.
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    let blocker = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    let recovery = dir.path().join("recovery");
    assert!(matches!(
        vault.change_master_password(OLD, NEW, &recovery),
        Err(VaultError::Save)
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        vault.reveal(&id).unwrap(),
        "Distinct synthetic login secret"
    );
    let copies: Vec<_> = fs::read_dir(recovery).unwrap().collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(
        fs::read(copies[0].as_ref().unwrap().path()).unwrap(),
        original
    );
    drop(blocker);
}
