//! Preserve the browser owner across hidden desktop startup. This is a UX hint,
//! never account authorization. No site or credential data belongs in this hint.
const PREFIX: &str = "--browser-startup-owner=";
const MAX_AGE_MS: u32 = 15_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptOwner {
    pub hwnd: isize,
    process_id: u32,
}

#[derive(Debug)]
pub enum RestoreFailure {
    Stale,
    WindowGone,
    WindowHidden,
    WindowMinimized,
    RootChanged,
    ProcessChanged,
    UnsupportedProcess,
    ForegroundChanged,
    ForegroundDenied,
}

impl RestoreFailure {
    pub fn stage(&self) -> &'static str {
        match self {
            Self::Stale => "pin_owner_stale",
            Self::WindowGone => "pin_owner_window_gone",
            Self::WindowHidden => "pin_owner_window_hidden",
            Self::WindowMinimized => "pin_owner_window_minimized",
            Self::RootChanged => "pin_owner_root_changed",
            Self::ProcessChanged => "pin_owner_process_changed",
            Self::UnsupportedProcess => "pin_owner_process_unsupported",
            Self::ForegroundChanged => "pin_owner_other_app_foreground",
            Self::ForegroundDenied => "pin_owner_foreground_denied",
        }
    }
}

#[derive(Clone, Copy)]
enum Foreground {
    Browser,
    Startup,
    None,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StartupOwner {
    request_id: String,
    owner: PromptOwner,
    captured_at: u32,
    // Retained for compatibility with the already-installed native host. Mouse
    // movement updates this global tick, so it cannot establish a focus change.
    input_at: u32,
}

fn valid_nonce(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn browser_executable(path: &str) -> bool {
    path.rsplit(['\\', '/']).next().is_some_and(|name| {
        ["brave.exe", "msedge.exe", "chrome.exe"]
            .iter()
            .any(|browser| name.eq_ignore_ascii_case(browser))
    })
}

impl StartupOwner {
    pub fn argument(&self) -> String {
        format!(
            "{PREFIX}{}:{}:{}:{}:{}",
            self.request_id,
            self.owner.hwnd,
            self.owner.process_id,
            self.captured_at,
            self.input_at
        )
    }

    pub fn parse(argument: &str) -> Option<Self> {
        let mut fields = argument.strip_prefix(PREFIX)?.split(':');
        let request_id = fields.next()?;
        let hwnd = fields.next()?.parse::<isize>().ok()?;
        let process_id = fields.next()?.parse::<u32>().ok()?;
        let captured_at = fields.next()?.parse().ok()?;
        let input_at = fields.next()?.parse().ok()?;
        if fields.next().is_some() || !valid_nonce(request_id) || hwnd <= 0 || process_id == 0 {
            return None;
        }
        Some(Self {
            request_id: request_id.into(),
            owner: PromptOwner { hwnd, process_id },
            captured_at,
            input_at,
        })
    }

    fn permits_restore(&self, request_id: &str, now: u32, foreground: Foreground) -> bool {
        self.matches_selection(request_id, now)
            && matches!(
                foreground,
                Foreground::Browser | Foreground::Startup | Foreground::None
            )
    }

    fn matches_selection(&self, request_id: &str, now: u32) -> bool {
        self.request_id == request_id && now.wrapping_sub(self.captured_at) <= MAX_AGE_MS
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            SystemInformation::GetTickCount,
            Threading::{
                GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{
                GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
                IsWindowVisible, SetForegroundWindow, GA_ROOTOWNER,
            },
        },
    };

    fn last_input() -> Option<u32> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: the structure has the required initialized size and lifetime.
        (unsafe { GetLastInputInfo(&mut info) } != 0).then_some(info.dwTime)
    }

    fn browser_process(process_id: u32) -> bool {
        // SAFETY: query-only handle, closed on every successful open.
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

    impl PromptOwner {
        fn foreground(self) -> Foreground {
            // SAFETY: query only. A newly-created Tauri/WebView window can own
            // foreground during startup even though the vault stays hidden.
            unsafe {
                let window = GetForegroundWindow();
                if window.is_null() {
                    return Foreground::None;
                }
                let root = GetAncestor(window, GA_ROOTOWNER);
                if root.is_null() {
                    return Foreground::Other;
                }
                let mut process_id = 0;
                if GetWindowThreadProcessId(root, &mut process_id) == 0 {
                    return Foreground::Other;
                }
                if root as isize == self.hwnd && process_id == self.process_id {
                    Foreground::Browser
                } else if process_id == GetCurrentProcessId() {
                    Foreground::Startup
                } else {
                    Foreground::Other
                }
            }
        }

        fn validate(self) -> Result<(), RestoreFailure> {
            let hwnd = self.hwnd as *mut std::ffi::c_void;
            let mut process_id = 0;
            // SAFETY: HWND queries only; handles may disappear, so validate each time.
            unsafe {
                if IsWindow(hwnd) == 0 {
                    return Err(RestoreFailure::WindowGone);
                }
                if IsWindowVisible(hwnd) == 0 {
                    return Err(RestoreFailure::WindowHidden);
                }
                if IsIconic(hwnd) != 0 {
                    return Err(RestoreFailure::WindowMinimized);
                }
                if GetAncestor(hwnd, GA_ROOTOWNER) != hwnd {
                    return Err(RestoreFailure::RootChanged);
                }
                if GetWindowThreadProcessId(hwnd, &mut process_id) == 0
                    || process_id != self.process_id
                {
                    return Err(RestoreFailure::ProcessChanged);
                }
                if !browser_process(process_id) {
                    return Err(RestoreFailure::UnsupportedProcess);
                }
                Ok(())
            }
        }

        pub fn capture() -> Option<Self> {
            // SAFETY: queries only, no ownership of HWND is acquired.
            unsafe {
                let root = GetAncestor(GetForegroundWindow(), GA_ROOTOWNER);
                if root.is_null() {
                    return None;
                }
                let mut process_id = 0;
                GetWindowThreadProcessId(root, &mut process_id);
                let owner = Self {
                    hwnd: root as isize,
                    process_id,
                };
                owner.validate().is_ok().then_some(owner)
            }
        }

        pub fn still_foreground(self) -> bool {
            Self::capture() == Some(self)
        }
    }

    impl StartupOwner {
        pub fn capture(request_id: &str) -> Option<Self> {
            if !valid_nonce(request_id) {
                return None;
            }
            Some(Self {
                request_id: request_id.into(),
                owner: PromptOwner::capture()?,
                // SAFETY: monotonic Windows tick query, with wrapping comparison.
                captured_at: unsafe { GetTickCount() },
                input_at: last_input()?,
            })
        }

        pub fn restore(self, request_id: &str) -> Result<PromptOwner, RestoreFailure> {
            let now = unsafe { GetTickCount() };
            if !self.matches_selection(request_id, now) {
                return Err(RestoreFailure::Stale);
            }
            self.owner.validate()?;
            let foreground = self.owner.foreground();
            if matches!(foreground, Foreground::Browser) {
                return Ok(self.owner);
            }
            // Check where focus actually went, not whether the mouse moved.
            // Another application (or another browser window) always cancels.
            if !self.permits_restore(request_id, now, foreground) {
                return Err(RestoreFailure::ForegroundChanged);
            }
            if !self.owner.still_foreground() {
                // SAFETY: still-live browser HWND validated above. Windows retains
                // final control over foreground permission. Never force via input.
                unsafe {
                    SetForegroundWindow(self.owner.hwnd as *mut std::ffi::c_void);
                }
            }
            if self.owner.still_foreground() {
                Ok(self.owner)
            } else {
                Err(RestoreFailure::ForegroundDenied)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const NONCE: &str = "0123456789abcdef0123456789abcdef";
    fn hint() -> StartupOwner {
        StartupOwner {
            request_id: NONCE.into(),
            owner: PromptOwner {
                hwnd: 1234,
                process_id: 5678,
            },
            captured_at: 100,
            input_at: 90,
        }
    }

    #[test]
    fn startup_hint_roundtrips_bounded_native_context() {
        assert_eq!(StartupOwner::parse(&hint().argument()), Some(hint()));
        for value in [
            "",
            "--browser-startup-owner=bad:1:2:3:4",
            "--browser-startup-owner=0123456789abcdef0123456789abcdef:0:2:3:4",
            "--browser-startup-owner=0123456789abcdef0123456789abcdef:1:0:3:4",
            "--browser-startup-owner=0123456789abcdef0123456789abcdef:1:2:3:4:extra",
        ] {
            assert!(StartupOwner::parse(value).is_none());
        }
    }
    #[test]
    fn restore_requires_exact_selection_fresh_startup_and_safe_foreground() {
        for foreground in [Foreground::Browser, Foreground::Startup, Foreground::None] {
            assert!(hint().permits_restore(NONCE, 5000, foreground));
            assert!(!hint().permits_restore("ffffffffffffffffffffffffffffffff", 5000, foreground));
            assert!(!hint().permits_restore(NONCE, 15101, foreground));
        }
        assert!(!hint().permits_restore(NONCE, 5000, Foreground::Other));
        let mut wrap = hint();
        wrap.captured_at = u32::MAX - 100;
        assert!(wrap.permits_restore(NONCE, 100, Foreground::Startup));
        assert!(!wrap.permits_restore(NONCE, 20000, Foreground::Startup));
    }
    #[test]
    fn mouse_activity_does_not_cancel_our_startup_window_but_other_apps_do() {
        let mut changed_input = hint();
        changed_input.input_at = 123456;
        assert!(changed_input.permits_restore(NONCE, 5000, Foreground::Startup));
        assert!(!changed_input.permits_restore(NONCE, 5000, Foreground::Other));
    }
    #[test]
    fn owner_process_must_be_supported_browser() {
        for path in [
            r"C:\Browser\brave.exe",
            r"C:\Browser\msedge.exe",
            r"C:\Browser\CHROME.EXE",
        ] {
            assert!(browser_executable(path));
        }
        for path in [r"C:\App\fakkarni.exe", r"C:\App\brave.exe.old", ""] {
            assert!(!browser_executable(path));
        }
    }
}
