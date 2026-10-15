//! Password rotation stays native; a successful disk commit is always reported.
use crate::{browser::BrowserState, hello::HelloState, Shared};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};
use tauri::{Emitter, State};
use zeroize::Zeroizing;

pub(super) struct CredentialsState {
    data_dir: PathBuf,
}
impl CredentialsState {
    pub(super) fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChangeReport {
    recovery_path: String,
    warnings: Vec<String>,
}

#[tauri::command]
pub(super) async fn change_master_password(
    state: State<'_, CredentialsState>,
    vault: State<'_, Shared>,
    browser: State<'_, Arc<BrowserState>>,
    hello: State<'_, HelloState>,
    app: tauri::AppHandle,
    current_password: String,
    new_password: String,
) -> Result<ChangeReport, String> {
    let current_password = Zeroizing::new(current_password);
    let new_password = Zeroizing::new(new_password);
    let data_dir = state.data_dir.clone();
    let vault = vault.inner().clone();
    let browser = browser.inner().clone();
    let gate = hello.credential_gate();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = gate.try_lock().map_err(|_| "Another PIN or password operation is in progress.")?;
        commit_change(&data_dir, &vault, &browser, epoch, &current_password, &new_password)
            .inspect(|_| { let _ = app.emit("vault-locked", ()); })
    }).await.map_err(|_| "Password change stopped unexpectedly. Check which password opens your vault before retrying.")?
}

fn commit_change(
    data_dir: &std::path::Path,
    vault: &Shared,
    browser: &BrowserState,
    epoch: u64,
    current_password: &str,
    new_password: &str,
) -> Result<ChangeReport, String> {
    let mut guard = vault.inner.lock().map_err(|_| "Vault unavailable.")?;
    if vault.blocked() || vault.epoch.load(Ordering::SeqCst) != epoch || guard.epoch != epoch {
        return Err("Password change cancelled by locking. Unlock and try again.".into());
    }
    let recovery_path = guard
        .vault
        .change_master_password(current_password, new_password, &data_dir.join("backups"))
        .map_err(|error| error.to_string())?;
    // Publication has succeeded. Cleanup failures are warnings, never a false
    // failure that might cause someone to continue relying on their old password.
    vault.epoch.fetch_add(1, Ordering::SeqCst);
    guard.epoch = vault.epoch.load(Ordering::SeqCst);
    let mut warnings = Vec::new();
    if browser.reset_after_restore().is_err() {
        warnings.push("Password changed, but browser metadata cleanup failed. Check local file permissions before restarting.".into());
    }
    let manager = fakkarni_hello_enrollment::Manager::new(
        data_dir.join("windows-hello.json"),
        data_dir.join("vault.kdbx"),
    );
    if let Err(error) = manager.disable() {
        warnings.push(format!("Password changed. PIN setup cleanup: {error}"));
    }
    Ok(ChangeReport {
        recovery_path: recovery_path.display().to_string(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GuardedVault, SharedState};
    use std::sync::{
        atomic::{AtomicBool, AtomicU64},
        Mutex,
    };
    const OLD: &str = "synthetic old master password";
    const NEW: &str = "synthetic replacement master password";
    fn setup(dir: &std::path::Path) -> (Shared, BrowserState) {
        let mut vault = fakkarni_vault::Vault::new(dir.join("vault.kdbx"));
        vault.create(OLD).unwrap();
        let state = Arc::new(SharedState {
            inner: Mutex::new(GuardedVault { vault, epoch: 0 }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        });
        std::fs::write(
            dir.join("browser-index.json"),
            r#"{"settings":{"showAccountsWhenLocked":true},"entries":[]}"#,
        )
        .unwrap();
        (state, BrowserState::new(dir.join("browser-index.json")))
    }
    #[test]
    fn success_revokes_session_removes_enrollment_and_resets_browser_index() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, browser) = setup(dir.path());
        // Invalid enrollment metadata must also be removed without calling Windows.
        std::fs::write(
            dir.path().join("windows-hello.json"),
            "invalid synthetic metadata",
        )
        .unwrap();
        let report = commit_change(dir.path(), &vault, &browser, 0, OLD, NEW).unwrap();
        assert!(report.warnings.is_empty());
        assert!(std::path::Path::new(&report.recovery_path).exists());
        assert!(!dir.path().join("windows-hello.json").exists());
        assert!(
            !std::fs::read_to_string(dir.path().join("browser-index.json"))
                .unwrap()
                .contains("true")
        );
        assert_eq!(vault.epoch.load(Ordering::SeqCst), 1);
        let mut guard = vault.inner.lock().unwrap();
        assert!(!guard.vault.status().unlocked);
        assert!(guard.vault.unlock(OLD).is_err());
        guard.vault.unlock(NEW).unwrap();
    }
    #[test]
    fn rejected_authentication_and_revoked_requests_preserve_metadata_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, browser) = setup(dir.path());
        let original = std::fs::read(dir.path().join("vault.kdbx")).unwrap();
        assert!(commit_change(dir.path(), &vault, &browser, 0, "wrong", NEW).is_err());
        assert!(vault.inner.lock().unwrap().vault.status().unlocked);
        vault.revoke();
        assert!(commit_change(dir.path(), &vault, &browser, 0, OLD, NEW).is_err());
        assert_eq!(
            original,
            std::fs::read(dir.path().join("vault.kdbx")).unwrap()
        );
        assert!(!dir.path().join("backups").exists());
        assert!(
            std::fs::read_to_string(dir.path().join("browser-index.json"))
                .unwrap()
                .contains("true")
        );
    }
    #[test]
    fn cleanup_failure_reports_success_with_warning_after_password_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, browser) = setup(dir.path());
        std::fs::create_dir(dir.path().join("windows-hello.json")).unwrap();
        let report = commit_change(dir.path(), &vault, &browser, 0, OLD, NEW).unwrap();
        assert_eq!(report.warnings.len(), 1);
        vault.inner.lock().unwrap().vault.unlock(NEW).unwrap();
    }
}
