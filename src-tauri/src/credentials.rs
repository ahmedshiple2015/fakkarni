//! Password rotation stays native; a successful disk commit is always reported.
use crate::{browser::BrowserState, hello::HelloState, Shared};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
};
use tauri::{Emitter, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
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
    Ok(finish_change(
        data_dir,
        vault,
        &mut guard,
        browser,
        Some(recovery_path),
    ))
}

fn finish_change(
    data_dir: &Path,
    vault: &Shared,
    guard: &mut crate::GuardedVault,
    browser: &BrowserState,
    recovery_path: Option<PathBuf>,
) -> ChangeReport {
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
    ChangeReport {
        recovery_path: recovery_path
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        warnings,
    }
}

fn local_path(path: tauri_plugin_dialog::FilePath) -> Result<PathBuf, String> {
    path.into_path()
        .ok()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| "Choose a local file.".into())
}

#[tauri::command]
pub(super) async fn export_recovery_key(
    vault: State<'_, Shared>,
    hello: State<'_, HelloState>,
    window: WebviewWindow,
    password: String,
) -> Result<Option<String>, String> {
    let password = Zeroizing::new(password);
    let vault = vault.inner().clone();
    let gate = hello.credential_gate();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = gate
            .try_lock()
            .map_err(|_| "Another credential operation is in progress.")?;
        let Some(file) = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Save encrypted recovery file")
            .add_filter("Fakkarni recovery", &["fkrec"])
            .set_file_name("Fakkarni-recovery.fkrec")
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = local_path(file)?;
        let key = crate::run_operation(&vault, epoch, |v| {
            v.export_recovery_envelope(&password, &path, || {
                !vault.blocked() && vault.epoch.load(Ordering::SeqCst) == epoch
            })
            .map_err(|e| e.to_string())
        })?;
        Ok(Some(key.to_string()))
    })
    .await
    .map_err(|_| "Could not create a recovery key.")?
}

#[tauri::command]
pub(super) async fn recover_master_password(
    state: State<'_, CredentialsState>,
    vault: State<'_, Shared>,
    browser: State<'_, Arc<BrowserState>>,
    hello: State<'_, HelloState>,
    window: WebviewWindow,
    recovery_key: Option<String>,
    new_password: String,
) -> Result<Option<ChangeReport>, String> {
    let app = window.app_handle().clone();
    let recovery_key = recovery_key.map(Zeroizing::new);
    let new_password = Zeroizing::new(new_password);
    if new_password.chars().count() < 12 || new_password.len() > 1024 {
        return Err("Use at least 12 characters and no more than 1,024 bytes.".into());
    }
    let hwnd = if recovery_key.is_none() {
        Some(crate::hello::window_handle(&window)?)
    } else {
        None
    };
    let data_dir = state.data_dir.clone();
    let vault = vault.inner().clone();
    let browser = browser.inner().clone();
    let manager = hello.manager();
    let gate = hello.credential_gate();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = gate
            .try_lock()
            .map_err(|_| "Another credential operation is in progress.")?;
        let allowed = || !vault.blocked() && vault.epoch.load(Ordering::SeqCst) == epoch;
        let stamp = crate::run_operation(&vault, epoch, |v| {
            if v.status().exists {
                v.recovery_stamp().map(Some).map_err(|e| e.to_string())
            } else {
                Ok(None)
            }
        })?;
        let (secret, source) = if let Some(key) = recovery_key {
            let Some(file) = window
                .dialog()
                .file()
                .set_parent(&window)
                .set_title("Choose your encrypted recovery file")
                .add_filter("Fakkarni recovery", &["fkrec"])
                .blocking_pick_file()
            else {
                return Ok(None);
            };
            let secret = fakkarni_vault::recovery::open_envelope(&local_path(file)?, &key)
                .map_err(|e| e.to_string())?;
            let source = if stamp.is_none() {
                let Some(file) = window
                    .dialog()
                    .file()
                    .set_parent(&window)
                    .set_title("Choose the matching encrypted vault backup")
                    .add_filter("KeePass vault", &["kdbx"])
                    .blocking_pick_file()
                else {
                    return Ok(None);
                };
                Some(local_path(file)?)
            } else {
                None
            };
            (secret, source)
        } else {
            if stamp.is_none() || !allowed() {
                return Err("No vault is available for PIN recovery.".into());
            }
            let secret = manager.unlock(hwnd.ok_or("PIN window unavailable.")?)?;
            (
                fakkarni_vault::recovery::RecoverySecret {
                    password: secret.password,
                    identity: secret.vault_id,
                    generation: None,
                },
                None,
            )
        };
        let mut guard = vault.inner.lock().map_err(|_| "Vault unavailable.")?;
        if guard.epoch != epoch || !allowed() {
            return Err("Recovery was cancelled by locking.".into());
        }
        let recovery_path = if let Some(stamp) = stamp {
            Some(
                guard
                    .vault
                    .recover_password(
                        &secret,
                        stamp,
                        &new_password,
                        &data_dir.join("backups"),
                        allowed,
                    )
                    .map_err(|e| e.to_string())?,
            )
        } else {
            guard
                .vault
                .recover_into_empty(
                    &source.ok_or("Choose a vault backup.")?,
                    &secret,
                    &new_password,
                    allowed,
                )
                .map_err(|e| e.to_string())?;
            None
        };
        let report = finish_change(&data_dir, &vault, &mut guard, &browser, recovery_path);
        let _ = app.emit("vault-locked", ());
        Ok(Some(report))
    })
    .await
    .map_err(|_| "Recovery stopped unexpectedly. Check which password opens your vault.")?
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
