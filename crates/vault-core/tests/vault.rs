use fakkarni_vault::{generate_password, EntryInput, Vault, VaultError};
use std::fs;

const MASTER: &str = "synthetic test master password";

#[test]
fn hello_reauthentication_keeps_session_and_stable_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let mut vault = Vault::new(path.clone());
    vault.create(MASTER).unwrap();
    let identity = vault.identity().unwrap();
    assert!(matches!(
        vault.verify_password("wrong synthetic password"),
        Err(VaultError::Unlock)
    ));
    assert!(vault.status().unlocked);
    vault.verify_password(MASTER).unwrap();
    vault.save_entry(&login()).unwrap();
    assert_eq!(vault.identity().unwrap(), identity);
    vault.lock();
    assert!(matches!(vault.identity(), Err(VaultError::Locked)));
    assert!(matches!(
        vault.verify_password(MASTER),
        Err(VaultError::Locked)
    ));
    vault.unlock(MASTER).unwrap();
    assert_eq!(vault.identity().unwrap(), identity);
    fs::write(&path, b"external synthetic replacement").unwrap();
    assert!(matches!(
        vault.verify_password(MASTER),
        Err(VaultError::Changed)
    ));
}
fn login() -> EntryInput {
    EntryInput {
        id: None,
        title: "Synthetic فكرني 🔐".into(),
        username: "مستخدم-synthetic@example.test".into(),
        url: "https://example.test/login".into(),
        password: "Synthetic-only-كلمة-289!🔑".into(),
        notes: "Synthetic private note\nملاحظة تجريبية — café".into(),
    }
}

#[test]
fn entry_roundtrip_edit_lock_wrong_password_and_encrypted_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let mut vault = Vault::new(path.clone());
    vault.create(MASTER).unwrap();
    let mut input = login();
    let id = vault.save_entry(&input).unwrap();
    assert_eq!(vault.list().unwrap().len(), 1);
    assert_eq!(vault.reveal(&id).unwrap(), input.password);
    input.id = Some(id.clone());
    input.password = "Synthetic replacement كلمة 🔑!".into();
    vault.save_entry(&input).unwrap();
    let disk = fs::read(&path).unwrap();
    for secret in [
        MASTER,
        input.title.as_str(),
        input.username.as_str(),
        input.password.as_str(),
        input.url.as_str(),
        input.notes.as_str(),
    ] {
        assert!(!disk
            .windows(secret.len())
            .any(|part| part == secret.as_bytes()));
    }
    vault.lock();
    assert!(matches!(vault.list(), Err(VaultError::Locked)));
    assert!(matches!(vault.reveal(&id), Err(VaultError::Locked)));
    assert!(matches!(
        vault.unlock("incorrect synthetic password"),
        Err(VaultError::Unlock)
    ));
    assert!(!vault.status().unlocked);
    vault.unlock(MASTER).unwrap();
    assert_eq!(vault.edit(&id).unwrap().password, input.password);
    let reopened = vault.edit(&id).unwrap();
    assert_eq!(reopened.title, input.title);
    assert_eq!(reopened.username, input.username);
    assert_eq!(reopened.notes, input.notes);
    assert_eq!(vault.list().unwrap()[0].id, id);
}

#[test]
fn changed_file_is_preserved_and_failed_edit_is_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let mut vault = Vault::new(path.clone());
    vault.create(MASTER).unwrap();
    let mut input = login();
    let id = vault.save_entry(&input).unwrap();
    let original_password = input.password.clone();
    input.id = Some(id.clone());
    input.password = "Must never be committed".into();
    fs::write(&path, b"external synthetic replacement").unwrap();
    assert!(matches!(vault.save_entry(&input), Err(VaultError::Changed)));
    assert_eq!(fs::read(&path).unwrap(), b"external synthetic replacement");
    assert_eq!(vault.reveal(&id).unwrap(), original_password);
    input.id = None;
    assert!(matches!(vault.save_entry(&input), Err(VaultError::Changed)));
    assert_eq!(vault.list().unwrap().len(), 1);
}

#[test]
fn second_writer_and_create_overwrite_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let mut first = Vault::new(path.clone());
    first.create(MASTER).unwrap();
    let original = fs::read(&path).unwrap();
    let mut second = Vault::new(path.clone());
    assert!(matches!(second.unlock(MASTER), Err(VaultError::InUse)));
    first.lock();
    assert!(matches!(second.create(MASTER), Err(VaultError::Exists)));
    assert_eq!(fs::read(&path).unwrap(), original);
    second.unlock(MASTER).unwrap();
}

#[test]
fn corrupted_encrypted_payload_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    let mut vault = Vault::new(path.clone());
    vault.create(MASTER).unwrap();
    vault.save_entry(&login()).unwrap();
    vault.lock();
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x40;
    fs::write(&path, bytes).unwrap();
    assert!(matches!(vault.unlock(MASTER), Err(VaultError::Unlock)));
    assert!(!vault.status().unlocked);
}

#[test]
fn generator_enforces_bounds_and_allowed_characters() {
    for length in [0, 15, 129, usize::MAX] {
        assert!(generate_password(length).is_err());
    }
    let alphabet =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-_=+[]{}:,.?";
    for length in [16, 32, 128] {
        let password = generate_password(length).unwrap();
        assert_eq!(password.len(), length);
        assert!(password.bytes().all(|byte| alphabet.contains(&byte)));
    }
}

/// Generates a synthetic interoperability fixture only; never uses a personal vault.
#[test]
#[ignore]
fn generate_synthetic_interop_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/interop/synthetic.kdbx");
    if path.exists() {
        fs::remove_file(&path).unwrap();
    }
    let mut vault = Vault::new(path);
    vault.create(MASTER).unwrap();
    vault.save_entry(&login()).unwrap();
}

/// Exercises the same read operations used by account suggestions/autofill,
/// followed by metadata-only edits, reauthentication, and encrypted reopen.
/// All inputs are synthetic and intentionally distinguish every field.
#[test]
fn browser_reads_and_metadata_edits_never_replace_password_with_username() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-browser-lifecycle.kdbx");
    let mut vault = Vault::new(path.clone());
    vault.create(MASTER).unwrap();
    let mut expected = Vec::new();
    for index in 0..4 {
        let mut input = EntryInput {
            id: None,
            title: format!("Synthetic account {index}"),
            username: format!("user-{index}@example.test"),
            password: format!("Secret-{index}-different-from-username-🔐-كلمة"),
            url: format!("https://example.test/login/{index}?not-for-index=private"),
            notes: format!("Private synthetic note {index}\nseparate from password"),
        };
        input.id = Some(vault.save_entry(&input).unwrap());
        expected.push(input);
    }
    for cycle in 0..4 {
        let before_reads = fs::read(&path).unwrap();
        for _ in 0..50 {
            let list = vault.list().unwrap();
            let index = vault.browser_index().unwrap();
            let matches = vault.matches("https://example.test").unwrap();
            assert_eq!(list.len(), expected.len());
            assert_eq!(index.len(), expected.len());
            assert_eq!(matches.len(), expected.len());
            for input in &expected {
                let id = input.id.as_ref().unwrap();
                assert_eq!(vault.reveal(id).unwrap(), input.password);
                let edited = vault.edit(id).unwrap();
                assert_eq!(edited.username, input.username);
                assert_eq!(edited.password, input.password);
                assert_eq!(edited.notes, input.notes);
                assert_ne!(edited.password, edited.username);
                assert_eq!(
                    matches
                        .iter()
                        .find(|entry| &entry.id == id)
                        .unwrap()
                        .username,
                    input.username
                );
            }
        }
        vault.verify_password(MASTER).unwrap();
        assert!(vault.verify_password("wrong synthetic master").is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            before_reads,
            "Read-only browser operations wrote the vault"
        );
        for input in &mut expected {
            let id = input.id.as_ref().unwrap().clone();
            let mut edited = vault.edit(&id).unwrap();
            edited.title = format!("Renamed synthetic {cycle} {id}");
            edited.username = format!("updated-{cycle}-{id}@example.test");
            edited.url = format!("https://example.test/changed/{cycle}");
            // A metadata-only edit submits the original password unchanged.
            vault.save_entry(&edited).unwrap();
            input.title.clone_from(&edited.title);
            input.username.clone_from(&edited.username);
            input.url.clone_from(&edited.url);
            assert_eq!(vault.reveal(&id).unwrap(), input.password);
        }
        vault.lock();
        vault.unlock(MASTER).unwrap();
        for input in &expected {
            let id = input.id.as_ref().unwrap();
            let reopened = vault.edit(id).unwrap();
            assert_eq!(reopened.title, input.title);
            assert_eq!(reopened.username, input.username);
            assert_eq!(reopened.password, input.password);
            assert_eq!(reopened.notes, input.notes);
            assert_eq!(reopened.url, input.url);
            assert_ne!(vault.reveal(id).unwrap(), input.username);
        }
    }
}

#[test]
fn metadata_only_save_ignores_autofilled_password_and_requires_explicit_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-password-preservation.kdbx");
    let mut vault = Vault::new(path);
    vault.create(MASTER).unwrap();
    let mut input = login();
    let id = vault.save_entry(&input).unwrap();
    let original = input.password.clone();
    input.id = Some(id.clone());
    input.title = "Changed title only".into();
    input.password.clone_from(&input.username);
    vault
        .save_entry_with_password_update(&input, false)
        .unwrap();
    assert_eq!(vault.reveal(&id).unwrap(), original);
    vault.lock();
    vault.unlock(MASTER).unwrap();
    assert_eq!(vault.reveal(&id).unwrap(), original);
    assert_ne!(vault.reveal(&id).unwrap(), input.username);
    input.password.clear();
    vault
        .save_entry_with_password_update(&input, false)
        .unwrap();
    assert_eq!(vault.reveal(&id).unwrap(), original);
    assert!(vault.save_entry_with_password_update(&input, true).is_err());
    assert_eq!(vault.reveal(&id).unwrap(), original);
    input.password = "Explicit new synthetic password".into();
    vault.save_entry_with_password_update(&input, true).unwrap();
    vault.lock();
    vault.unlock(MASTER).unwrap();
    assert_eq!(vault.reveal(&id).unwrap(), input.password);
    input.id = None;
    input.password.clear();
    assert!(vault
        .save_entry_with_password_update(&input, false)
        .is_err());
    assert!(vault.save_entry_with_password_update(&input, true).is_err());
    assert_eq!(vault.list().unwrap().len(), 1);
}
