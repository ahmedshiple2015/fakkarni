//! Native file pickers and one-use restore previews. No frontend filesystem access.
use crate::{browser::BrowserState, Shared};
use fakkarni_vault::PreparedRestore;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroizing;

const PREVIEW_LIFETIME: Duration = Duration::from_secs(120);
const CANCELLED: &str =
    "Restore preview expired or was cancelled by locking. Choose the backup again.";

struct Pending {
    token: String,
    prepared: PreparedRestore,
    epoch: u64,
    created: Instant,
}
pub struct BackupState {
    gate: Arc<Mutex<()>>,
    pending: Mutex<Option<Pending>>,
    data_dir: PathBuf,
}
impl BackupState {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            gate: Arc::new(Mutex::new(())),
            pending: Mutex::new(None),
            data_dir,
        }
    }
    pub fn expire(&self) {
        if let Ok(mut pending) = self.pending.try_lock() {
            if pending
                .as_ref()
                .is_some_and(|p| p.created.elapsed() >= PREVIEW_LIFETIME)
            {
                *pending = None;
            }
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupReport {
    file_name: String,
    path: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    token: String,
    entry_count: usize,
    file_name: String,
    has_existing_vault: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    entry_count: usize,
    recovery_path: Option<String>,
    warnings: Vec<String>,
}
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Backup.kdbx".into())
}
fn current(vault: &Shared, epoch: u64) -> Result<(), String> {
    if vault.blocked() || vault.epoch.load(Ordering::SeqCst) != epoch {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}
fn selected_path(value: tauri_plugin_dialog::FilePath) -> Result<PathBuf, String> {
    let path = value
        .into_path()
        .map_err(|_| "Choose a local .kdbx file.")?;
    if !path.is_absolute() {
        return Err("Choose a local .kdbx file.".into());
    }
    Ok(path)
}

#[tauri::command]
pub async fn create_backup(
    state: State<'_, Arc<BackupState>>,
    vault: State<'_, Shared>,
    window: WebviewWindow,
) -> Result<Option<BackupReport>, String> {
    let state = state.inner().clone();
    let vault = vault.inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = state
            .gate
            .try_lock()
            .map_err(|_| "Another backup or restore is in progress.")?;
        crate::run_operation(&vault, epoch, |v| {
            v.identity().map_err(|e| e.to_string())?;
            Ok(())
        })?;
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let Some(selected) = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Save encrypted Fakkarni backup")
            .add_filter("Encrypted KeePass database", &["kdbx"])
            .set_file_name(format!("Fakkarni-backup-{suffix}.kdbx"))
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = selected_path(selected)?;
        // Export is encrypted and non-overwriting. Once it succeeds, report its
        // creation even if Windows locks during the final disk operation.
        let mut guard = vault.inner.lock().map_err(|_| "Vault unavailable.")?;
        current(&vault, epoch)?;
        if guard.epoch != epoch {
            return Err(CANCELLED.into());
        }
        guard
            .vault
            .export_backup(&path)
            .map_err(|e| e.to_string())?;
        Ok(Some(BackupReport {
            file_name: file_name(&path),
            path: path.display().to_string(),
        }))
    })
    .await
    .map_err(|_| "Couldn't create the encrypted backup.")?
}

#[tauri::command]
pub async fn prepare_restore(
    state: State<'_, Arc<BackupState>>,
    vault: State<'_, Shared>,
    window: WebviewWindow,
    password: String,
) -> Result<Option<RestorePreview>, String> {
    let password = Zeroizing::new(password);
    if password.is_empty() || password.len() > 1024 {
        return Err("Enter the backup's master password.".into());
    }
    let state = state.inner().clone();
    let vault = vault.inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = state
            .gate
            .try_lock()
            .map_err(|_| "Another backup or restore is in progress.")?;
        current(&vault, epoch)?;
        *state.pending.lock().map_err(|_| "Restore unavailable.")? = None;
        let Some(selected) = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Choose an encrypted Fakkarni backup")
            .add_filter("Encrypted KeePass database", &["kdbx"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = selected_path(selected)?;
        let prepared = crate::run_operation(&vault, epoch, |v| {
            v.prepare_restore(&path, &password)
                .map_err(|e| e.to_string())
        })?;
        let token = uuid::Uuid::new_v4().to_string();
        let report = RestorePreview {
            token: token.clone(),
            entry_count: prepared.entries(),
            file_name: file_name(&path),
            has_existing_vault: prepared.replaces_existing(),
        };
        current(&vault, epoch)?;
        *state.pending.lock().map_err(|_| "Restore unavailable.")? = Some(Pending {
            token,
            prepared,
            epoch,
            created: Instant::now(),
        });
        Ok(Some(report))
    })
    .await
    .map_err(|_| "Couldn't validate the encrypted backup.")?
}

#[tauri::command]
pub fn cancel_restore(state: State<'_, Arc<BackupState>>, token: String) -> Result<(), String> {
    let mut pending = state.pending.lock().map_err(|_| "Restore unavailable.")?;
    if pending.as_ref().is_some_and(|p| p.token == token) {
        *pending = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn commit_restore(
    state: State<'_, Arc<BackupState>>,
    vault: State<'_, Shared>,
    browser: State<'_, Arc<BrowserState>>,
    app: tauri::AppHandle,
    token: String,
) -> Result<RestoreReport, String> {
    let state = state.inner().clone();
    let vault = vault.inner().clone();
    let browser = browser.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = state
            .gate
            .try_lock()
            .map_err(|_| "Another backup or restore is in progress.")?;
        let pending = {
            let mut slot = state.pending.lock().map_err(|_| "Restore unavailable.")?;
            if !slot
                .as_ref()
                .is_some_and(|p| p.token == token && p.created.elapsed() < PREVIEW_LIFETIME)
            {
                if slot
                    .as_ref()
                    .is_some_and(|p| p.created.elapsed() >= PREVIEW_LIFETIME)
                {
                    *slot = None;
                }
                return Err(CANCELLED.into());
            }
            slot.take().ok_or(CANCELLED)?
        };
        commit_prepared(&state, &vault, &browser, pending).inspect(|_| {
            let _ = app.emit("vault-locked", ());
        })
    })
    .await
    .map_err(|_| {
        "The restore operation stopped unexpectedly. Check your local vault before retrying."
    })?
}

fn commit_prepared(
    state: &BackupState,
    vault: &Shared,
    browser: &BrowserState,
    pending: Pending,
) -> Result<RestoreReport, String> {
    let mut guard = vault.inner.lock().map_err(|_| "Vault unavailable.")?;
    current(vault, pending.epoch)?;
    if guard.epoch != pending.epoch {
        return Err(CANCELLED.into());
    }
    // The commit can publish the new encrypted file. Do not discard a successful
    // result on a later epoch change: cleanup must run and the UI must know it succeeded.
    let committed = guard
        .vault
        .commit_restore(pending.prepared, &state.data_dir.join("backups"))
        .map_err(|e| e.to_string())?;
    guard.vault.lock();
    vault.epoch.fetch_add(1, Ordering::SeqCst);
    guard.epoch = vault.epoch.load(Ordering::SeqCst);
    let mut warnings = Vec::new();
    if let Err(error) = browser.reset_after_restore() {
        warnings.push(error);
    }
    let hello = fakkarni_hello_enrollment::Manager::new(
        state.data_dir.join("windows-hello.json"),
        state.data_dir.join("vault.kdbx"),
    );
    if let Err(error) = hello.disable() {
        warnings.push(format!("Restore completed. {error}"));
    }
    Ok(RestoreReport {
        entry_count: committed.entries,
        recovery_path: committed.recovery_path.map(|p| p.display().to_string()),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GuardedVault, SharedState};
    use std::sync::atomic::{AtomicBool, AtomicU64};
    const MASTER: &str = "synthetic backup test master";

    fn shared(vault: fakkarni_vault::Vault) -> Shared {
        Arc::new(SharedState {
            inner: Mutex::new(GuardedVault { vault, epoch: 0 }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        })
    }
    fn pending(vault: &Shared, source: &Path, epoch: u64) -> Pending {
        Pending {
            token: uuid::Uuid::new_v4().to_string(),
            prepared: vault
                .inner
                .lock()
                .unwrap()
                .vault
                .prepare_restore(source, MASTER)
                .unwrap(),
            epoch,
            created: Instant::now(),
        }
    }
    fn source(dir: &Path) -> PathBuf {
        let path = dir.join("synthetic-source.kdbx");
        let mut vault = fakkarni_vault::Vault::new(path.clone());
        vault.create(MASTER).unwrap();
        vault.lock();
        path
    }
    #[test]
    fn restore_into_first_launch_locks_and_clears_old_browser_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let source = source(dir.path());
        let data = dir.path().join("new-app-data");
        std::fs::create_dir_all(&data).unwrap();
        let state = BackupState::new(data.clone());
        let vault = shared(fakkarni_vault::Vault::new(data.join("vault.kdbx")));
        let browser_path = data.join("browser-index.json");
        std::fs::write(
            &browser_path,
            r#"{"settings":{"showAccountsWhenLocked":true},"entries":[]}"#,
        )
        .unwrap();
        let browser = BrowserState::new(browser_path.clone());
        let report =
            commit_prepared(&state, &vault, &browser, pending(&vault, &source, 0)).unwrap();
        assert_eq!(report.entry_count, 0);
        assert!(report.recovery_path.is_none());
        assert!(report.warnings.is_empty());
        assert_eq!(vault.epoch.load(Ordering::SeqCst), 1);
        assert!(!vault.inner.lock().unwrap().vault.status().unlocked);
        assert!(!std::fs::read_to_string(browser_path)
            .unwrap()
            .contains("true"));
        vault.inner.lock().unwrap().vault.unlock(MASTER).unwrap();
    }
    #[test]
    fn lock_after_preview_prevents_commit_and_preserves_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let source = source(dir.path());
        let target = dir.path().join("vault.kdbx");
        let state = BackupState::new(dir.path().into());
        let vault = shared(fakkarni_vault::Vault::new(target.clone()));
        let browser_path = dir.path().join("browser-index.json");
        let metadata = r#"{"settings":{"showAccountsWhenLocked":true},"entries":[]}"#;
        std::fs::write(&browser_path, metadata).unwrap();
        let browser = BrowserState::new(browser_path.clone());
        let prepared = pending(&vault, &source, 0);
        vault.revoke();
        assert!(commit_prepared(&state, &vault, &browser, prepared).is_err());
        assert!(!target.exists());
        assert_eq!(std::fs::read_to_string(browser_path).unwrap(), metadata);
    }
    #[test]
    fn expired_preview_is_released_by_background_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let source = source(dir.path());
        let vault = shared(fakkarni_vault::Vault::new(dir.path().join("vault.kdbx")));
        let state = BackupState::new(dir.path().into());
        let mut candidate = pending(&vault, &source, 0);
        candidate.created = Instant::now() - PREVIEW_LIFETIME;
        *state.pending.lock().unwrap() = Some(candidate);
        state.expire();
        assert!(state.pending.lock().unwrap().is_none());
    }
}
