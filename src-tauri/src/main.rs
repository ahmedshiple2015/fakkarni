#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use fakkarni_vault::{EntryInput, EntrySummary, Status, Vault};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager, State};
use zeroize::Zeroizing;

struct GuardedVault {
    vault: Vault,
    epoch: u64,
}
struct SharedState {
    inner: Mutex<GuardedVault>,
    epoch: AtomicU64,
    os_locked: AtomicBool,
    suspended: AtomicBool,
}
type Shared = Arc<SharedState>;

impl SharedState {
    fn blocked(&self) -> bool {
        self.os_locked.load(Ordering::SeqCst) || self.suspended.load(Ordering::SeqCst)
    }
    // Revoke immediately; never block the Windows message loop on a KDF or save.
    fn revoke(&self) {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut inner) = self.inner.try_lock() {
            inner.vault.lock();
            inner.epoch = epoch;
        }
    }
}

async fn operation<T: Send + 'static>(
    vault: Shared,
    f: impl FnOnce(&mut Vault) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let epoch = vault.epoch.load(Ordering::SeqCst);
    let worker = vault.clone();
    let result = tauri::async_runtime::spawn_blocking(move || run_operation(&worker, epoch, f))
        .await
        .map_err(|_| "Vault operation failed.".to_owned())?;
    if vault.epoch.load(Ordering::SeqCst) != epoch || vault.blocked() {
        return Err("Vault request cancelled by locking. Unlock again.".to_owned());
    }
    result
}

fn run_operation<T>(
    vault: &SharedState,
    epoch: u64,
    f: impl FnOnce(&mut Vault) -> Result<T, String>,
) -> Result<T, String> {
    let mut inner = vault
        .inner
        .lock()
        .map_err(|_| "Vault unavailable. Restart Fakkarni.".to_owned())?;
    let current = vault.epoch.load(Ordering::SeqCst);
    if inner.epoch != current {
        inner.vault.lock();
        inner.epoch = current;
    }
    if epoch != current || vault.blocked() {
        return Err("Vault request cancelled by locking. Unlock again.".to_owned());
    }
    let result = f(&mut inner.vault);
    if vault.epoch.load(Ordering::SeqCst) != epoch || vault.blocked() {
        inner.vault.lock();
        inner.epoch = vault.epoch.load(Ordering::SeqCst);
        return Err("Vault request cancelled by locking. Unlock again.".to_owned());
    }
    result
}

#[cfg(test)]
mod revocation_tests {
    use super::*;
    fn state() -> SharedState {
        SharedState {
            inner: Mutex::new(GuardedVault {
                vault: Vault::new("unused-test-vault.kdbx".into()),
                epoch: 0,
            }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        }
    }
    #[test]
    fn queued_request_is_rejected_after_lock() {
        let state = state();
        state.revoke();
        assert!(run_operation(&state, 0, |_| -> Result<(), String> {
            panic!("stale operation executed")
        })
        .is_err());
    }
    #[test]
    fn lock_during_operation_discards_result() {
        let state = state();
        assert!(run_operation(&state, 0, |_| {
            state.revoke();
            Ok("secret")
        })
        .is_err());
    }
    #[test]
    fn os_lock_rejects_new_requests() {
        let state = state();
        state.os_locked.store(true, Ordering::SeqCst);
        assert!(run_operation(&state, 0, |_| -> Result<(), String> {
            panic!("operation executed while Windows locked")
        })
        .is_err());
    }
}

#[tauri::command]
async fn vault_status(state: State<'_, Shared>) -> Result<Status, String> {
    operation(state.inner().clone(), |v| Ok(v.status())).await
}

#[tauri::command]
async fn create_vault(state: State<'_, Shared>, password: String) -> Result<(), String> {
    let password = Zeroizing::new(password);
    operation(state.inner().clone(), move |v| {
        v.create(&password).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn unlock_vault(state: State<'_, Shared>, password: String) -> Result<(), String> {
    let password = Zeroizing::new(password);
    operation(state.inner().clone(), move |v| {
        v.unlock(&password).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn lock_vault(state: State<'_, Shared>, app: tauri::AppHandle) -> Result<(), String> {
    state.revoke();
    let _ = app.emit("vault-locked", ());
    Ok(())
}

#[tauri::command]
async fn touch_vault(state: State<'_, Shared>) -> Result<(), String> {
    operation(state.inner().clone(), |v| {
        v.touch().map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn list_entries(state: State<'_, Shared>) -> Result<Vec<EntrySummary>, String> {
    operation(state.inner().clone(), |v| {
        v.list().map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn edit_entry(state: State<'_, Shared>, id: String) -> Result<EntryInput, String> {
    operation(state.inner().clone(), move |v| {
        v.edit(&id).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn reveal_password(state: State<'_, Shared>, id: String) -> Result<String, String> {
    operation(state.inner().clone(), move |v| {
        v.reveal(&id).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn save_entry(state: State<'_, Shared>, input: EntryInput) -> Result<String, String> {
    operation(state.inner().clone(), move |v| {
        v.save_entry(&input).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
fn generate_password(length: usize) -> Result<String, String> {
    fakkarni_vault::generate_password(length).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let path = app.path().app_local_data_dir()?.join("vault.kdbx");
            let vault = Arc::new(SharedState {
                inner: Mutex::new(GuardedVault {
                    vault: Vault::new(path),
                    epoch: 0,
                }),
                epoch: AtomicU64::new(0),
                os_locked: AtomicBool::new(false),
                suspended: AtomicBool::new(false),
            });
            app.manage(vault.clone());
            #[cfg(windows)]
            windows_lock::start(vault.clone(), app.handle().clone())
                .map_err(std::io::Error::other)?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut previously_unlocked = false;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if let Ok(mut inner) = vault.inner.lock() {
                        let epoch = vault.epoch.load(Ordering::SeqCst);
                        if inner.epoch != epoch || vault.blocked() {
                            inner.vault.lock();
                            inner.epoch = epoch;
                        }
                        let unlocked = inner.vault.status().unlocked;
                        if previously_unlocked && !unlocked {
                            vault.epoch.fetch_add(1, Ordering::SeqCst);
                            inner.epoch = vault.epoch.load(Ordering::SeqCst);
                            let _ = handle.emit("vault-locked", ());
                        }
                        previously_unlocked = unlocked;
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            create_vault,
            unlock_vault,
            lock_vault,
            touch_vault,
            list_entries,
            edit_entry,
            reveal_password,
            save_entry,
            generate_password
        ])
        .run(tauri::generate_context!())
        .expect("Couldn't start Fakkarni");
}

#[cfg(windows)]
mod windows_lock {
    use super::*;
    use std::sync::OnceLock;
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            RemoteDesktop::{WTSRegisterSessionNotification, WTSUnRegisterSessionNotification},
        },
        UI::WindowsAndMessaging::*,
    };

    static CONTEXT: OnceLock<(Shared, tauri::AppHandle)> = OnceLock::new();

    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if let Some((vault, app)) = CONTEXT.get() {
            let revoke = if message == WM_WTSSESSION_CHANGE {
                match w {
                    2 | 4 | 6 | 7 => {
                        vault.os_locked.store(true, Ordering::SeqCst);
                        true
                    }
                    8 => {
                        vault.os_locked.store(false, Ordering::SeqCst);
                        true
                    }
                    _ => false,
                }
            } else if message == WM_POWERBROADCAST {
                match w {
                    4 => {
                        vault.suspended.store(true, Ordering::SeqCst);
                        true
                    }
                    6 | 7 | 18 => {
                        vault.suspended.store(false, Ordering::SeqCst);
                        true
                    }
                    _ => false,
                }
            } else {
                false
            };
            if revoke {
                vault.revoke();
                let _ = app.emit("vault-locked", ());
            }
        }
        unsafe { DefWindowProcW(hwnd, message, w, l) }
    }

    pub fn start(vault: Shared, app: tauri::AppHandle) -> Result<(), String> {
        CONTEXT
            .set((vault, app))
            .map_err(|_| "Session monitor already started.".to_owned())?;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || unsafe {
            let name: Vec<u16> = "FakkarniSessionMonitor\0".encode_utf16().collect();
            let instance = GetModuleHandleW(std::ptr::null());
            let mut class: WNDCLASSW = std::mem::zeroed();
            class.lpfnWndProc = Some(procedure);
            class.hInstance = instance;
            class.lpszClassName = name.as_ptr();
            if RegisterClassW(&class) == 0 {
                let _ = sender.send(Err("Couldn't register Windows session monitor.".to_owned()));
                return;
            }
            let window = CreateWindowExW(
                0,
                name.as_ptr(),
                name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if window.is_null() || WTSRegisterSessionNotification(window, 0) == 0 {
                if !window.is_null() {
                    DestroyWindow(window);
                }
                let _ = sender.send(Err("Couldn't monitor Windows lock events.".to_owned()));
                return;
            }
            let _ = sender.send(Ok(()));
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            WTSUnRegisterSessionNotification(window);
            DestroyWindow(window);
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "Windows session monitor timed out.".to_owned())?
    }
}
