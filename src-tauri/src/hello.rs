use super::{run_operation, Shared};
use fakkarni_hello_enrollment::Manager;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc, Mutex},
};
use tauri::{State, WebviewWindow};
use zeroize::Zeroizing;

pub(super) struct HelloState {
    gate: Arc<Mutex<()>>,
    enrollment_path: PathBuf,
    vault_path: PathBuf,
}

impl HelloState {
    pub(super) fn credential_gate(&self) -> Arc<Mutex<()>> {
        self.gate.clone()
    }
    pub(super) fn new(enrollment_path: PathBuf, vault_path: PathBuf) -> Self {
        Self {
            gate: Arc::new(Mutex::new(())),
            enrollment_path,
            vault_path,
        }
    }
}

#[derive(Serialize)]
pub(super) struct HelloStatus {
    available: bool,
    enabled: bool,
    reason: Option<String>,
}

fn window_handle(window: &WebviewWindow) -> Result<isize, String> {
    #[cfg(windows)]
    {
        Ok(window
            .hwnd()
            .map_err(|_| "Couldn't attach the Windows Hello prompt.".to_owned())?
            .0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("Windows Hello is supported only on Windows.".into())
    }
}

#[tauri::command]
pub(super) async fn hello_status(state: State<'_, HelloState>) -> Result<HelloStatus, String> {
    // Status reads only bounded encrypted metadata, never keys or a prompt.
    let manager = Manager::new(state.enrollment_path.clone(), state.vault_path.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let enabled = match manager.enabled() {
            Ok(value) => value,
            Err(_) => return HelloStatus { available: false, enabled: true, reason: Some("Saved Windows Hello setup is invalid. Use your master password, then disable Windows Hello and set it up again.".into()) },
        };
        match fakkarni_windows_hello::availability() {
            Ok(true) => HelloStatus { available: true, enabled, reason: None },
            Ok(false) => HelloStatus { available: false, enabled, reason: Some("Set up Windows Hello in Windows Settings to use this feature. Enrollment also requires compatible TPM-backed key protection.".into()) },
            Err(_) => HelloStatus { available: false, enabled, reason: Some("Windows Hello is unavailable. You can continue using your master password.".into()) },
        }
    }).await.map_err(|_| "Couldn't check Windows Hello availability.".to_owned())
}

fn current(vault: &Shared, epoch: u64) -> Result<(), String> {
    if vault.blocked() || vault.epoch.load(Ordering::SeqCst) != epoch {
        Err("Windows Hello request cancelled because the vault was locked. Try again after unlocking.".into())
    } else {
        Ok(())
    }
}

fn unlock_verified_secret(
    vault: &Shared,
    epoch: u64,
    secret: fakkarni_hello_enrollment::UnlockedSecret,
) -> Result<(), String> {
    run_operation(vault, epoch, |v| {
        v.unlock(&secret.password).map_err(|e| e.to_string())?;
        match v.identity() {
            Ok(identity) if identity == secret.vault_id => Ok(()),
            _ => {
                v.lock();
                Err("Windows Hello enrollment belongs to a different vault. Use your master password.".into())
            }
        }
    })
}

#[tauri::command]
pub(super) async fn enable_hello(
    state: State<'_, Shared>,
    hello: State<'_, HelloState>,
    window: WebviewWindow,
    password: String,
) -> Result<(), String> {
    let password = Zeroizing::new(password);
    let hwnd = window_handle(&window)?;
    let vault = state.inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    // Moving only paths into the worker avoids retaining a Tauri State borrow.
    let enrollment_path = hello.enrollment_path.clone();
    let vault_path = hello.vault_path.clone();
    let manager_state = hello.gate.clone();
    let worker = vault.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _guard = manager_state
            .try_lock()
            .map_err(|_| "Another Windows Hello request is in progress.".to_owned())?;
        current(&worker, epoch)?;
        let manager = Manager::new(enrollment_path, vault_path);
        if manager.enabled()? {
            return Err(
                "Windows Hello is already enabled. Disable it before setting it up again.".into(),
            );
        }
        let identity = run_operation(&worker, epoch, |v| {
            v.verify_password(&password).map_err(|e| e.to_string())?;
            v.identity().map_err(|e| e.to_string())
        })?;
        current(&worker, epoch)?;
        manager.enroll(&password, &identity, hwnd)?;
        let valid = current(&worker, epoch).and_then(|_| {
            run_operation(&worker, epoch, |v| {
                if v.identity().map_err(|e| e.to_string())? != identity {
                    return Err("The vault changed during setup.".into());
                }
                Ok(())
            })
        });
        if valid.is_err() {
            let _ = manager.disable();
        }
        valid
    })
    .await
    .map_err(|_| "Windows Hello setup failed. Your master password still works.".to_owned())?;
    current(&vault, epoch)?;
    result
}

#[tauri::command]
pub(super) async fn unlock_hello(
    state: State<'_, Shared>,
    hello: State<'_, HelloState>,
    window: WebviewWindow,
) -> Result<(), String> {
    let hwnd = window_handle(&window)?;
    let vault = state.inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    let manager_state = hello.gate.clone();
    let manager = Manager::new(hello.enrollment_path.clone(), hello.vault_path.clone());
    let worker = vault.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _guard = manager_state
            .try_lock()
            .map_err(|_| "Another Windows Hello request is in progress.".to_owned())?;
        current(&worker, epoch)?;
        let secret = manager.unlock(hwnd)?;
        // A lock event during the OS prompt revokes the result before KDF/unlock.
        unlock_verified_secret(&worker, epoch, secret)
    })
    .await
    .map_err(|_| "Windows Hello unlock failed. Use your master password.".to_owned())?;
    current(&vault, epoch)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GuardedVault, SharedState};
    use std::sync::atomic::{AtomicBool, AtomicU64};
    const MASTER: &str = "synthetic test recovery password";

    fn setup() -> (tempfile::TempDir, Shared, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fakkarni_vault::Vault::new(dir.path().join("synthetic.kdbx"));
        vault.create(MASTER).unwrap();
        let id = vault.identity().unwrap();
        vault.lock();
        let state = Arc::new(SharedState {
            inner: Mutex::new(GuardedVault { vault, epoch: 0 }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        });
        (dir, state, id)
    }
    fn secret(id: String) -> fakkarni_hello_enrollment::UnlockedSecret {
        fakkarni_hello_enrollment::UnlockedSecret {
            password: Zeroizing::new(MASTER.into()),
            vault_id: id,
        }
    }
    #[test]
    fn hello_unlock_binds_to_vault_identity_and_keeps_mismatch_locked() {
        let (_dir, state, id) = setup();
        assert!(unlock_verified_secret(&state, 0, secret("wrong-vault".into())).is_err());
        assert!(!state.inner.lock().unwrap().vault.status().unlocked);
        unlock_verified_secret(&state, 0, secret(id)).unwrap();
        assert!(state.inner.lock().unwrap().vault.status().unlocked);
    }
    #[test]
    fn prompt_result_cannot_unlock_after_a_lock_event() {
        let (_dir, state, id) = setup();
        state.revoke();
        assert!(unlock_verified_secret(&state, 0, secret(id)).is_err());
        assert!(!state.inner.lock().unwrap().vault.status().unlocked);
    }
}

#[tauri::command]
pub(super) async fn disable_hello(
    state: State<'_, Shared>,
    hello: State<'_, HelloState>,
) -> Result<(), String> {
    let vault = state.inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    let manager_state = hello.gate.clone();
    let manager = Manager::new(hello.enrollment_path.clone(), hello.vault_path.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = manager_state
            .try_lock()
            .map_err(|_| "Another Windows Hello request is in progress.".to_owned())?;
        run_operation(&vault, epoch, |v| {
            v.identity().map_err(|e| e.to_string())?;
            manager.disable()
        })
    })
    .await
    .map_err(|_| "Couldn't disable Windows Hello.".to_owned())?
}
