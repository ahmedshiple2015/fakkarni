#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod browser;
mod hello;
mod lifecycle;

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

    #[test]
    fn browser_suggestions_are_scoped_metadata_and_honor_locks() {
        use fakkarni_browser_bridge::{Request, Status};
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::new(dir.path().join("synthetic.kdbx"));
        vault.create("synthetic browser test master").unwrap();
        let input = EntryInput {
            id: None,
            title: "Example account".into(),
            username: "synthetic-user".into(),
            password: "must-never-reach-browser".into(),
            notes: "private synthetic note".into(),
            url: "https://example.test/login?private=do-not-send".into(),
        };
        vault.save_entry(&input).unwrap();
        let state = SharedState {
            inner: Mutex::new(GuardedVault { vault, epoch: 0 }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        };
        let browser = browser::BrowserState::new(dir.path().join("browser.json"));
        let request = || Request::Match {
            version: 1,
            origin: "https://example.test".into(),
        };
        let response = browser::handle(&state, &browser, None, request());
        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.matches.len(), 1);
        assert_eq!(response.matches[0].url, "https://example.test");
        assert_eq!(response.matches[0].username, "synthetic-user");
        assert!(browser::handle(
            &state,
            &browser,
            None,
            Request::Match {
                version: 1,
                origin: "https://example.test.attacker.test".into()
            }
        )
        .matches
        .is_empty());
        assert_eq!(
            browser::handle(
                &state,
                &browser,
                None,
                Request::Match {
                    version: 1,
                    origin: "https://example.test/login".into()
                }
            )
            .status,
            Status::Error
        );
        {
            let _busy = state.inner.lock().unwrap();
            assert_eq!(
                browser::handle(&state, &browser, None, request()).status,
                Status::Unavailable
            );
        }
        state.revoke();
        assert_eq!(
            browser::handle(&state, &browser, None, request()).status,
            Status::Locked
        );
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
        let mut input = v.edit(&id).map_err(|e| e.to_string())?;
        use zeroize::Zeroize;
        input.password.zeroize();
        Ok(input)
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
async fn save_entry(
    state: State<'_, Shared>,
    input: EntryInput,
    replace_password: Option<bool>,
) -> Result<String, String> {
    let replace_password = replace_password.unwrap_or(input.id.is_none());
    operation(state.inner().clone(), move |v| {
        v.save_entry_with_password_update(&input, replace_password)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
fn generate_password(length: usize) -> Result<String, String> {
    fakkarni_vault::generate_password(length).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_entry(
    state: State<'_, Shared>,
    browser: State<'_, Arc<browser::BrowserState>>,
    id: String,
) -> Result<Option<String>, String> {
    let vault = state.inner().clone();
    let browser = browser.inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = vault.inner.lock().map_err(|_| "Vault unavailable.")?;
        let current = vault.epoch.load(Ordering::SeqCst);
        if guard.epoch != current {
            guard.vault.lock();
            guard.epoch = current;
        }
        if epoch != current || vault.blocked() {
            return Err("Delete cancelled because the vault was locked.".into());
        }
        guard
            .vault
            .delete_entry(&id)
            .map_err(|error| error.to_string())?;
        let warning = browser.forget_entry(&id).err();
        // Deletion is already committed. Report that accurately if an OS lock
        // arrives during persistence, while still revoking the session.
        if vault.epoch.load(Ordering::SeqCst) != epoch || vault.blocked() {
            guard.vault.lock();
            guard.epoch = vault.epoch.load(Ordering::SeqCst);
        }
        Ok(warning)
    })
    .await
    .map_err(|_| "Delete stopped unexpectedly. Reopen the vault to check its state.".to_owned())?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args
                .iter()
                .any(|arg| arg == "--autostart" || arg == "--browser-background")
            {
                lifecycle::show_main(app);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let backup_state = Arc::new(backup::BackupState::new(data_dir.clone()));
            app.manage(backup_state.clone());
            let path = data_dir.join("vault.kdbx");
            app.manage(hello::HelloState::new(
                data_dir.join("windows-hello.json"),
                path.clone(),
            ));
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
            let browser_state = Arc::new(browser::BrowserState::new(
                data_dir.join("browser-index.json"),
            ));
            app.manage(browser_state.clone());
            #[cfg(windows)]
            {
                let browser_vault = vault.clone();
                let browser_handle = app.handle().clone();
                let browser_state = browser_state.clone();
                let bridge = fakkarni_browser_bridge::start_server(move |request| {
                    browser::handle(
                        &browser_vault,
                        &browser_state,
                        Some(&browser_handle),
                        request,
                    )
                })?;
                app.manage(bridge);
            }
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
                        if !unlocked {
                            browser_state.revoke_ready();
                        }
                        if unlocked {
                            browser_state.refresh(&mut inner.vault);
                        }
                        if previously_unlocked && !unlocked {
                            vault.epoch.fetch_add(1, Ordering::SeqCst);
                            inner.epoch = vault.epoch.load(Ordering::SeqCst);
                            let _ = handle.emit("vault-locked", ());
                        }
                        previously_unlocked = unlocked;
                    }
                    browser_state.complete_after_unlock(&vault);
                    backup_state.expire();
                }
            });
            lifecycle::init(app)?;
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
            delete_entry,
            generate_password,
            hello::hello_status,
            hello::enable_hello,
            hello::unlock_hello,
            hello::disable_hello,
            browser::browser_pending,
            browser::reject_browser_fill,
            browser::browser_settings,
            browser::set_browser_settings,
            lifecycle::get_app_settings,
            lifecycle::set_app_settings,
            backup::create_backup,
            backup::prepare_restore,
            backup::cancel_restore,
            backup::commit_restore
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
