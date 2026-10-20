//! Browser-triggered Windows Hello unlock without revealing the desktop window.
use crate::{browser::BrowserState, hello::HelloState, Shared};
use std::sync::{atomic::Ordering, Arc};
use tauri::{Emitter, Manager};

const HELLO_CANCELLED: &str = "Windows Hello was cancelled.";

#[cfg(any(windows, test))]
fn browser_executable(path: &str) -> bool {
    path.rsplit(['\\', '/']).next().is_some_and(|name| {
        ["brave.exe", "msedge.exe", "chrome.exe"]
            .iter()
            .any(|browser| name.eq_ignore_ascii_case(browser))
    })
}

#[cfg(windows)]
fn browser_process(process_id: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };
    // SAFETY: the process handle is query-only, used synchronously, and always
    // closed. This is a foreground UX check, not authorization for a vault entry.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if process.is_null() {
            return false;
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let valid = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) != 0;
        CloseHandle(process);
        valid && browser_executable(&String::from_utf16_lossy(&path[..length as usize]))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Completed,
    Fallback,
    Cancelled,
    Stale,
}

/// Keep provider calls injectable so cancellation and stale-result handling can
/// be verified without invoking Windows Hello or opening any native window.
fn attempt<T>(
    current: impl Fn() -> bool,
    enabled: impl FnOnce() -> Result<bool, String>,
    prompt: impl FnOnce() -> Result<T, String>,
    commit: impl FnOnce(T) -> Result<(), String>,
) -> Outcome {
    if !current() {
        return Outcome::Stale;
    }
    if !matches!(enabled(), Ok(true)) {
        return if current() {
            Outcome::Fallback
        } else {
            Outcome::Stale
        };
    }
    if !current() {
        return Outcome::Stale;
    }
    match prompt() {
        Ok(secret) => {
            if !current() {
                return Outcome::Stale;
            }
            match commit(secret) {
                Ok(()) => Outcome::Completed,
                Err(_) if current() => Outcome::Fallback,
                Err(_) => Outcome::Stale,
            }
        }
        Err(error) if error == HELLO_CANCELLED => Outcome::Cancelled,
        Err(_) if current() => Outcome::Fallback,
        Err(_) => Outcome::Stale,
    }
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct PromptOwner {
    hwnd: isize,
    process_id: u32,
}

#[cfg(windows)]
impl PromptOwner {
    fn capture() -> Option<Self> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
            IsWindowVisible, GA_ROOTOWNER,
        };
        // SAFETY: these queries do not take ownership of the returned HWND.
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground.is_null() {
                return None;
            }
            let root = GetAncestor(foreground, GA_ROOTOWNER);
            if root.is_null()
                || IsWindow(root) == 0
                || IsWindowVisible(root) == 0
                || IsIconic(root) != 0
            {
                return None;
            }
            let mut process_id = 0;
            if GetWindowThreadProcessId(root, &mut process_id) == 0
                || process_id == 0
                || !browser_process(process_id)
            {
                return None;
            }
            Some(Self {
                hwnd: root as isize,
                process_id,
            })
        }
    }
    fn still_foreground(self) -> bool {
        Self::capture()
            .is_some_and(|now| now.hwnd == self.hwnd && now.process_id == self.process_id)
    }
}

/// Claim exactly one prompt for the selected pending account, while the browser
/// still owns the foreground. Neither acquiring its HWND nor successful Hello
/// authentication calls show_main.
pub(super) fn begin(app: &tauri::AppHandle, request_id: String) {
    let vault = app.state::<Shared>().inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    let browser = app.state::<Arc<BrowserState>>().inner().clone();
    let queued_app = app.clone();
    let queued_request = request_id.clone();
    // The IPC thread can run while Tauri's event loop is still starting. Claim
    // and capture the prompt owner only once that loop can dispatch UI work.
    if app
        .run_on_main_thread(move || {
            if !vault.blocked() && vault.epoch.load(Ordering::SeqCst) == epoch {
                begin_ready(&queued_app, queued_request, epoch);
            } else {
                queued_app
                    .state::<Arc<BrowserState>>()
                    .cancel_waiting(&queued_request);
            }
        })
        .is_err()
    {
        browser.cancel_waiting(&request_id);
    }
}

fn begin_ready(app: &tauri::AppHandle, request_id: String, epoch: u64) {
    let vault = app.state::<Shared>().inner().clone();
    let browser = app.state::<Arc<BrowserState>>().inner().clone();
    if !browser.claim_unlock(&request_id) {
        return;
    }
    #[cfg(windows)]
    let Some(owner) = PromptOwner::capture() else {
        browser.cancel_waiting(&request_id);
        return;
    };
    let hello = app.state::<HelloState>();
    let manager = hello.manager();
    let gate = hello.credential_gate();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(_guard) = gate.try_lock() else {
            // An existing Hello operation owns the verification UI. Do not open
            // a second prompt or surface the vault as an accidental fallback.
            browser.cancel_waiting(&request_id);
            return;
        };
        let current = || {
            !vault.blocked()
                && vault.epoch.load(Ordering::SeqCst) == epoch
                && browser.is_waiting(&request_id)
        };
        let outcome = attempt(
            current,
            || {
                if !manager.enabled()? {
                    return Ok(false);
                }
                fakkarni_windows_hello::availability()
            },
            || {
                #[cfg(windows)]
                {
                    // The user may have changed applications while availability
                    // was checked. Treat a stale owner as cancellation, rather
                    // than showing a prompt attached to an unrelated window.
                    if !owner.still_foreground() {
                        return Err(HELLO_CANCELLED.into());
                    }
                    manager.unlock(owner.hwnd)
                }
                #[cfg(not(windows))]
                {
                    Err("Windows Hello is unavailable.".into())
                }
            },
            |secret| {
                crate::hello::unlock_browser_secret(&vault, epoch, &browser, &request_id, secret)
            },
        );
        match outcome {
            Outcome::Completed => {
                if vault.blocked() || vault.epoch.load(Ordering::SeqCst) != epoch {
                    browser.cancel_waiting(&request_id);
                    return;
                }
                browser.complete_after_unlock(&vault);
                let _ = app.emit("browser-vault-unlocked", ());
            }
            Outcome::Fallback if current() => {
                #[cfg(windows)]
                if !owner.still_foreground() {
                    browser.cancel_waiting(&request_id);
                    return;
                }
                crate::lifecycle::show_main(&app);
                let _ = app.emit("browser-fill-request", ());
            }
            Outcome::Fallback | Outcome::Cancelled | Outcome::Stale => {
                browser.cancel_waiting(&request_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn prompt_owner_must_be_a_supported_browser_process() {
        for path in [
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Browser\CHROME.EXE",
        ] {
            assert!(browser_executable(path));
        }
        for path in [
            r"C:\App\fakkarni.exe",
            r"C:\App\brave.exe.old",
            r"C:\brave.exe\other.exe",
            "",
        ] {
            assert!(!browser_executable(path));
        }
    }

    #[test]
    fn enabled_hello_commits_without_desktop_fallback() {
        let committed = Cell::new(false);
        assert_eq!(
            attempt(
                || true,
                || Ok(true),
                || Ok("synthetic secret"),
                |secret| {
                    assert_eq!(secret, "synthetic secret");
                    committed.set(true);
                    Ok(())
                }
            ),
            Outcome::Completed
        );
        assert!(committed.get());
    }
    #[test]
    fn missing_or_unavailable_hello_falls_back_without_prompting() {
        for enabled in [Ok(false), Err("Synthetic unavailable provider".into())] {
            assert_eq!(
                attempt::<()>(
                    || true,
                    || enabled,
                    || panic!("Unavailable Hello prompted"),
                    |_| panic!("Unavailable Hello committed")
                ),
                Outcome::Fallback
            );
        }
    }
    #[test]
    fn cancelled_prompt_does_not_fall_back_or_commit() {
        assert_eq!(
            attempt::<()>(
                || true,
                || Ok(true),
                || Err(HELLO_CANCELLED.into()),
                |_| panic!("Cancelled prompt committed")
            ),
            Outcome::Cancelled
        );
    }
    #[test]
    fn stale_or_cancelled_request_never_commits_a_returned_secret() {
        let current = Cell::new(true);
        assert_eq!(
            attempt(
                || current.get(),
                || Ok(true),
                || {
                    current.set(false);
                    Ok("synthetic stale secret")
                },
                |_| panic!("Stale prompt result committed")
            ),
            Outcome::Stale
        );
        assert_eq!(
            attempt::<()>(
                || false,
                || panic!("Stale request checked provider"),
                || panic!("Stale request prompted"),
                |_| panic!("Stale request committed")
            ),
            Outcome::Stale
        );
    }
    #[test]
    fn provider_or_commit_error_falls_back_only_for_current_request() {
        assert_eq!(
            attempt::<()>(
                || true,
                || Ok(true),
                || Err("Synthetic provider error".into()),
                |_| panic!("Failed provider committed")
            ),
            Outcome::Fallback
        );
        assert_eq!(
            attempt(
                || true,
                || Ok(true),
                || Ok(()),
                |_| Err("Synthetic invalid binding".into())
            ),
            Outcome::Fallback
        );
        let current = Cell::new(true);
        assert_eq!(
            attempt::<()>(
                || current.get(),
                || Ok(true),
                || {
                    current.set(false);
                    Err("Synthetic provider error".into())
                },
                |_| panic!("Failed provider committed")
            ),
            Outcome::Stale
        );
    }
    #[test]
    fn unused_secret_is_dropped_when_request_changes_after_prompt() {
        struct Secret<'a>(&'a RefCell<bool>);
        impl Drop for Secret<'_> {
            fn drop(&mut self) {
                *self.0.borrow_mut() = true;
            }
        }
        let dropped = RefCell::new(false);
        let current = Cell::new(true);
        assert_eq!(
            attempt(
                || current.get(),
                || Ok(true),
                || {
                    current.set(false);
                    Ok(Secret(&dropped))
                },
                |_| panic!("Revoked secret committed")
            ),
            Outcome::Stale
        );
        assert!(*dropped.borrow());
    }
}
