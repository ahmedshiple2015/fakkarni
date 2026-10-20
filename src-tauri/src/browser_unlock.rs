//! Browser-triggered Windows Hello unlock without revealing the desktop window.
use crate::{browser::BrowserState, hello::HelloState, Shared};
use std::sync::{atomic::Ordering, Arc};
use tauri::{Emitter, Manager};

const HELLO_CANCELLED: &str = "Windows Hello was cancelled.";

#[cfg(windows)]
use fakkarni_browser_bridge::prompt_owner::{PromptOwner, StartupOwner};

#[cfg(windows)]
fn capture_owner(request_id: &str) -> Option<PromptOwner> {
    use std::sync::{Mutex, OnceLock};
    // Read once and consume once. CLI context alone never starts a prompt: the
    // validated, exact pending account request must reach begin_ready first.
    static STARTUP: OnceLock<Mutex<Option<StartupOwner>>> = OnceLock::new();
    let hint = STARTUP
        .get_or_init(|| Mutex::new(std::env::args().find_map(|arg| StartupOwner::parse(&arg))))
        .lock()
        .ok()?
        .take();
    if let Some(hint) = hint {
        crate::unlock_trace::record("pin_startup_owner_received");
        hint.restore(request_id)
    } else {
        PromptOwner::capture()
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

/// Claim exactly one prompt for the selected pending account, while the browser
/// still owns the foreground. Neither acquiring its HWND nor successful Hello
/// authentication calls show_main.
pub(super) fn begin(app: &tauri::AppHandle, request_id: String) {
    crate::unlock_trace::record("pin_dispatch_queued");
    let vault = app.state::<Shared>().inner().clone();
    let epoch = vault.epoch.load(Ordering::SeqCst);
    let browser = app.state::<Arc<BrowserState>>().inner().clone();
    let queued_app = app.clone();
    let queued_request = request_id.clone();
    // The IPC thread can run while Tauri's event loop is still starting. Claim
    // and capture the prompt owner only once that loop can dispatch UI work.
    if app
        .run_on_main_thread(move || {
            crate::unlock_trace::record("pin_dispatch_running");
            if !vault.blocked() && vault.epoch.load(Ordering::SeqCst) == epoch {
                begin_ready(&queued_app, queued_request, epoch);
            } else {
                crate::unlock_trace::record("pin_dispatch_revoked");
                queued_app
                    .state::<Arc<BrowserState>>()
                    .cancel_waiting(&queued_request);
            }
        })
        .is_err()
    {
        crate::unlock_trace::record("pin_dispatch_failed");
        browser.cancel_waiting(&request_id);
    }
}

fn begin_ready(app: &tauri::AppHandle, request_id: String, epoch: u64) {
    let vault = app.state::<Shared>().inner().clone();
    let browser = app.state::<Arc<BrowserState>>().inner().clone();
    if !browser.claim_unlock(&request_id) {
        crate::unlock_trace::record("pin_claim_rejected");
        return;
    }
    crate::unlock_trace::record("pin_claimed");
    #[cfg(windows)]
    let Some(owner) = capture_owner(&request_id) else {
        crate::unlock_trace::record("pin_owner_invalid");
        browser.cancel_waiting(&request_id);
        return;
    };
    crate::unlock_trace::record("pin_owner_captured");
    let hello = app.state::<HelloState>();
    let manager = hello.manager();
    let gate = hello.credential_gate();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::unlock_trace::record("pin_worker_started");
        let Ok(_guard) = gate.try_lock() else {
            crate::unlock_trace::record("pin_gate_busy");
            // An existing Hello operation owns the verification UI. Do not open
            // a second prompt or surface the vault as an accidental fallback.
            browser.cancel_waiting(&request_id);
            return;
        };
        let current = || {
            if vault.blocked() {
                crate::unlock_trace::record("pin_request_os_blocked");
                return false;
            }
            if vault.epoch.load(Ordering::SeqCst) != epoch {
                crate::unlock_trace::record("pin_request_epoch_changed");
                return false;
            }
            if !browser.is_waiting(&request_id) {
                crate::unlock_trace::record("pin_request_not_waiting");
                return false;
            }
            true
        };
        let outcome = attempt(
            current,
            || {
                crate::unlock_trace::record("pin_enrollment_check");
                if !manager.enabled().inspect_err(|error| {
                    crate::unlock_trace::failure("pin_enrollment_error", error)
                })? {
                    crate::unlock_trace::record("pin_not_enrolled");
                    return Ok(false);
                }
                crate::unlock_trace::record("pin_enrolled");
                fakkarni_windows_hello::availability()
                    .inspect(|available| {
                        crate::unlock_trace::record(if *available {
                            "pin_provider_available"
                        } else {
                            "pin_provider_unavailable"
                        })
                    })
                    .inspect_err(|error| crate::unlock_trace::failure("pin_provider_error", error))
            },
            || {
                #[cfg(windows)]
                {
                    // The user may have changed applications while availability
                    // was checked. Treat a stale owner as cancellation, rather
                    // than showing a prompt attached to an unrelated window.
                    if !owner.still_foreground() {
                        crate::unlock_trace::record("pin_owner_changed_before_prompt");
                        return Err(HELLO_CANCELLED.into());
                    }
                    crate::unlock_trace::record("pin_prompt_started");
                    manager
                        .unlock(owner.hwnd)
                        .inspect(|_| crate::unlock_trace::record("pin_prompt_verified"))
                        .inspect_err(|error| {
                            crate::unlock_trace::failure("pin_prompt_error", error)
                        })
                }
                #[cfg(not(windows))]
                {
                    Err("Windows Hello is unavailable.".into())
                }
            },
            |secret| {
                crate::unlock_trace::record("pin_vault_unlock_started");
                crate::hello::unlock_browser_secret(&vault, epoch, &browser, &request_id, secret)
                    .inspect_err(|error| {
                        crate::unlock_trace::failure("pin_vault_unlock_error", error)
                    })
            },
        );
        crate::unlock_trace::record(match outcome {
            Outcome::Completed => "pin_completed",
            Outcome::Fallback => "pin_fallback",
            Outcome::Cancelled => "pin_cancelled",
            Outcome::Stale => "pin_stale",
        });
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
