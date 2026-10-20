//! Chromium Native Messaging host. Never opens or decrypts a vault.
use fakkarni_browser_bridge::{
    read_frame, request_desktop, write_frame, Match, Request, Response, Status, MAX_FRAME_BYTES,
    MAX_NATIVE_REQUEST_BYTES,
};
use fakkarni_vault::account_index::{read_index, Index};
use std::io;
use std::io::Read;

const EXTENSION_ORIGIN: &str = "chrome-extension://kieggakkiiegglikoiojkbfabpmmahep/";

fn error_response() -> Response {
    Response {
        version: 1,
        status: Status::Error,
        matches: Vec::new(),
        credential: None,
    }
}

fn handle(caller: Option<&str>, input: &mut impl io::Read) -> Response {
    handle_with(
        caller,
        input,
        request_desktop,
        read_cached_index,
        launch_desktop,
    )
}

fn handle_with(
    caller: Option<&str>,
    input: &mut impl io::Read,
    mut desktop: impl FnMut(&Request) -> io::Result<Response>,
    mut cache: impl FnMut() -> Option<Index>,
    mut launch: impl FnMut(&Request) -> io::Result<()>,
) -> Response {
    if caller != Some(EXTENSION_ORIGIN) {
        return error_response();
    }
    let request = read_frame(input, MAX_NATIVE_REQUEST_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok());
    let Some(request) = request else {
        return error_response();
    };
    if request.validate().is_err() || fakkarni_vault::validate_origin(request.origin()).is_err() {
        return error_response();
    }
    if let Ok(response) = desktop(&request) {
        return response;
    }
    if let Request::Match { origin, .. } = &request {
        // Merely opening the extension must not start the desktop. Only explicit
        // opt-in account metadata is available here, never encrypted-vault data.
        return cached_matches(origin, cache());
    }
    if !matches!(request, Request::BeginFill { .. }) || launch(&request).is_err() {
        return Response::empty(Status::Unavailable);
    }
    // Cold Tauri/WebView startup can exceed three seconds on an ordinary PC.
    // Keep this same explicit account request alive while the desktop gets ready.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(response) = desktop(&request) {
            return response;
        }
    }
    Response::empty(Status::Unavailable)
}

fn read_cached_index() -> Option<Index> {
    let host = std::env::current_exe().ok()?;
    let path = host.parent()?.parent()?.join("browser-index.json");
    read_index(&path)
}

fn cached_matches(origin: &str, index: Option<Index>) -> Response {
    let mut response = Response::empty(Status::Locked);
    if let Some(index) = index.filter(|index| index.settings.show_accounts_when_locked) {
        response.matches = index
            .entries
            .into_iter()
            .filter(|entry| entry.origin == origin)
            .take(50)
            .map(|entry| Match {
                id: entry.id,
                title: entry.title,
                username: entry.username,
                url: entry.origin,
            })
            .collect();
        // Cached metadata bypasses the desktop pipe's response-size guard.
        // Enforce the same wire bound here, including JSON escaping overhead.
        while serde_json::to_vec(&response).is_ok_and(|bytes| bytes.len() > MAX_FRAME_BYTES) {
            response.matches.pop();
        }
    }
    response
}

fn launch_desktop(request: &Request) -> io::Result<()> {
    let Request::BeginFill { request_id, .. } = request else {
        return Err(io::Error::other(
            "Only an account selection can launch the desktop",
        ));
    };
    #[cfg(windows)]
    let owner = fakkarni_browser_bridge::prompt_owner::StartupOwner::capture(request_id);
    #[cfg(not(windows))]
    let _ = request_id;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Config {
        desktop_exe_full_path: String,
    }
    let config_path = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("Missing host directory"))?
        .join("desktop-path.json");
    let file = std::fs::File::open(config_path)?;
    let mut bytes = Vec::new();
    io::Read::take(file, 8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err(io::Error::other("Invalid desktop configuration"));
    }
    let config: Config = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::other("Invalid desktop configuration"))?;
    let path = std::path::PathBuf::from(config.desktop_exe_full_path);
    if !path.is_absolute()
        || path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("exe"))
        || !path.is_file()
    {
        return Err(io::Error::other("Invalid desktop executable"));
    }
    let mut command = std::process::Command::new(path);
    command
        .arg("--browser-background")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Chromium can pass inheritable native-message pipes. Even with null child
        // stdio, an unrelated inheritable handle would otherwise keep the browser's
        // pipe open for the desktop's entire lifetime. Keep our handles open, but
        // remove inheritance before creating any child process.
        prevent_protocol_handle_inheritance()?;
        command.creation_flags(0x08000000);
    }
    #[cfg(windows)]
    if let Some(owner) = owner {
        command.arg(owner.argument());
    }
    let child = command.spawn()?;
    #[cfg(windows)]
    {
        // Grant only this child our foreground permission, if Windows allows it.
        // The desktop still validates the browser owner and unchanged user input.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(child.id());
        }
    }
    #[cfg(not(windows))]
    let _ = child;
    Ok(())
}

#[cfg(windows)]
fn clear_handle_inheritance(handle: windows_sys::Win32::Foundation::HANDLE) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{
        SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Ok(());
    }
    // SAFETY: this modifies only the inheritance flag; it neither closes nor
    // changes access to the live standard handle supplied by Windows.
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
#[cfg(windows)]
fn prevent_protocol_handle_inheritance() -> io::Result<()> {
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for selector in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: selectors are Windows constants. Absent handles are ignored.
        clear_handle_inheritance(unsafe { GetStdHandle(selector) })?;
    }
    Ok(())
}

fn main() {
    let caller = std::env::args().nth(1);
    let response = handle(caller.as_deref(), &mut io::stdin().lock());
    // Stdout is exclusively the browser's framed JSON protocol. No credential logging.
    if let Ok(bytes) = serde_json::to_vec(&response) {
        let bytes = zeroize::Zeroizing::new(bytes);
        let _ = write_frame(&mut io::stdout().lock(), &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fakkarni_vault::account_index::{BrowserSettings, IndexEntry};

    fn synthetic_cache() -> Index {
        Index {
            settings: BrowserSettings {
                show_accounts_when_locked: true,
            },
            entries: (0..60)
                .map(|id| IndexEntry {
                    id: format!("synthetic-{id}"),
                    title: "Synthetic account".into(),
                    username: "synthetic@example.test".into(),
                    origin: "https://example.test".into(),
                })
                .collect(),
        }
    }
    fn framed(request: &Request) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &serde_json::to_vec(request).unwrap()).unwrap();
        bytes
    }
    fn unreachable(_: &Request) -> io::Result<Response> {
        Err(io::Error::other("Synthetic offline desktop"))
    }

    #[test]
    fn closed_desktop_match_uses_only_opt_in_origin_cache_without_launching() {
        let request = Request::Match {
            version: 1,
            origin: "https://example.test".into(),
        };
        let bytes = framed(&request);
        let response = handle_with(
            Some(EXTENSION_ORIGIN),
            &mut bytes.as_slice(),
            unreachable,
            || Some(synthetic_cache()),
            |_| panic!("A match request launched the desktop"),
        );
        assert_eq!(response.status, Status::Locked);
        assert_eq!(response.matches.len(), 50);
        assert!(response.credential.is_none());
        assert!(response
            .matches
            .iter()
            .all(|entry| entry.url == "https://example.test"));
        for origin in [
            "https://example.test.attacker.test",
            "https://sub.example.test",
            "https://example.test:8443",
        ] {
            let bytes = framed(&Request::Match {
                version: 1,
                origin: origin.into(),
            });
            let response = handle_with(
                Some(EXTENSION_ORIGIN),
                &mut bytes.as_slice(),
                unreachable,
                || Some(synthetic_cache()),
                |_| panic!("Match launched desktop"),
            );
            assert!(response.matches.is_empty());
        }
        for index in [
            None,
            Some(Index::default()),
            Some(Index {
                settings: BrowserSettings::default(),
                entries: synthetic_cache().entries,
            }),
        ] {
            let response = handle_with(
                Some(EXTENSION_ORIGIN),
                &mut bytes.as_slice(),
                unreachable,
                || {
                    index.as_ref().map(|index| Index {
                        settings: index.settings.clone(),
                        entries: index.entries.clone(),
                    })
                },
                |_| panic!("Match launched desktop"),
            );
            assert_eq!(response.status, Status::Locked);
            assert!(response.matches.is_empty());
            assert!(response.credential.is_none());
        }
    }

    #[test]
    fn closed_desktop_begin_launches_then_forwards_exact_request_without_cache() {
        let request = Request::BeginFill {
            version: 1,
            origin: "https://example.test".into(),
            id: "synthetic-account".into(),
            request_id: "0123456789abcdef0123456789abcdef".into(),
        };
        let bytes = framed(&request);
        let mut attempts = 0;
        let mut launches = 0;
        let response = handle_with(
            Some(EXTENSION_ORIGIN),
            &mut bytes.as_slice(),
            |received| {
                assert_eq!(received, &request);
                attempts += 1;
                if attempts == 1 {
                    unreachable(received)
                } else {
                    Ok(Response::empty(Status::Pending))
                }
            },
            || panic!("Begin fill read account cache"),
            |launched| {
                assert_eq!(launched, &request);
                launches += 1;
                Ok(())
            },
        );
        assert_eq!(launches, 1);
        assert_eq!(attempts, 2);
        assert_eq!(response.status, Status::Pending);
        assert!(response.credential.is_none());
    }

    #[test]
    fn cold_start_keeps_first_selection_past_the_old_three_second_deadline() {
        let request = Request::BeginFill {
            version: 1,
            origin: "https://example.test".into(),
            id: "synthetic-account".into(),
            request_id: "0123456789abcdef0123456789abcdef".into(),
        };
        let bytes = framed(&request);
        let started = std::time::Instant::now();
        let mut launches = 0;
        let response = handle_with(
            Some(EXTENSION_ORIGIN),
            &mut bytes.as_slice(),
            |received| {
                assert_eq!(received, &request);
                if started.elapsed() < std::time::Duration::from_millis(3200) {
                    unreachable(received)
                } else {
                    Ok(Response::empty(Status::Pending))
                }
            },
            || panic!("Begin fill read account cache"),
            |launched| {
                assert_eq!(launched, &request);
                launches += 1;
                Ok(())
            },
        );
        assert_eq!(launches, 1);
        assert_eq!(response.status, Status::Pending);
        assert!(response.credential.is_none());
    }

    #[test]
    fn unauthorized_or_invalid_requests_never_read_cache_or_launch() {
        for (caller, request) in [
            (
                Some("chrome-extension://unapproved/"),
                Request::Match {
                    version: 1,
                    origin: "https://example.test".into(),
                },
            ),
            (
                Some(EXTENSION_ORIGIN),
                Request::Match {
                    version: 1,
                    origin: "https://example.test/login".into(),
                },
            ),
            (
                Some(EXTENSION_ORIGIN),
                Request::Match {
                    version: 2,
                    origin: "https://example.test".into(),
                },
            ),
        ] {
            let bytes = framed(&request);
            let response = handle_with(
                caller,
                &mut bytes.as_slice(),
                |_| panic!("Invalid caller reached desktop"),
                || panic!("Invalid caller read cache"),
                |_| panic!("Invalid caller launched desktop"),
            );
            assert_eq!(response.status, Status::Error);
        }
    }

    #[test]
    fn available_desktop_takes_precedence_and_poll_never_launches() {
        let bytes = framed(&Request::Match {
            version: 1,
            origin: "https://example.test".into(),
        });
        let response = handle_with(
            Some(EXTENSION_ORIGIN),
            &mut bytes.as_slice(),
            |_| Ok(Response::empty(Status::Ok)),
            || panic!("Live desktop fell back to cache"),
            |_| panic!("Live desktop was launched again"),
        );
        assert_eq!(response.status, Status::Ok);
        let bytes = framed(&Request::PollFill {
            version: 1,
            origin: "https://example.test".into(),
            request_id: "0123456789abcdef0123456789abcdef".into(),
        });
        let response = handle_with(
            Some(EXTENSION_ORIGIN),
            &mut bytes.as_slice(),
            unreachable,
            || panic!("Poll read cache"),
            |_| panic!("Poll launched desktop"),
        );
        assert_eq!(response.status, Status::Unavailable);
    }

    #[test]
    fn cached_matches_fit_native_frame_after_json_escaping() {
        let mut index = synthetic_cache();
        for entry in &mut index.entries {
            entry.title = "\u{0001}".repeat(1024);
            entry.username = "\u{0002}".repeat(2048);
        }
        let response = cached_matches("https://example.test", Some(index));
        assert!(!response.matches.is_empty());
        assert!(response.matches.len() < 50);
        assert!(response.credential.is_none());
        let bytes = serde_json::to_vec(&response).unwrap();
        assert!(bytes.len() <= MAX_FRAME_BYTES);
        write_frame(&mut Vec::new(), &bytes).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn clearing_inheritance_keeps_the_protocol_handle_valid() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        };
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let handle = file.as_raw_handle();
        assert_ne!(
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) },
            0
        );
        clear_handle_inheritance(handle).unwrap();
        let mut flags = 0;
        assert_ne!(unsafe { GetHandleInformation(handle, &mut flags) }, 0);
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
        assert!(file.metadata().unwrap().len() > 0);
        clear_handle_inheritance(std::ptr::null_mut()).unwrap();
        clear_handle_inheritance(INVALID_HANDLE_VALUE).unwrap();
    }
    #[test]
    fn native_caller_matches_the_extension_manifest_identity() {
        let identity: serde_json::Value =
            serde_json::from_str(include_str!("../../../extension/identity.json")).unwrap();
        assert_eq!(
            EXTENSION_ORIGIN,
            format!(
                "chrome-extension://{}/",
                identity["extensionId"].as_str().unwrap()
            )
        );
        assert_eq!(identity["nativeHost"], "org.fakkarni.bridge");
    }
    #[test]
    fn rejects_unapproved_extension_without_consuming_input() {
        let mut bytes: &[u8] = &[1, 2, 3, 4];
        let response = handle(Some("chrome-extension://unapproved/"), &mut bytes);
        assert!(matches!(response.status, Status::Error));
        assert_eq!(bytes, [1, 2, 3, 4]);
    }
    #[test]
    fn rejects_malformed_or_unsupported_requests_before_desktop_access() {
        for body in [
            r#"{"version":2,"action":"match","origin":"https://example.com"}"#,
            r#"{"version":1,"action":"match","origin":"http://example.com"}"#,
            r#"{"version":1,"action":"match","origin":"https://example.com/login"}"#,
            r#"{"version":1,"action":"reveal","id":"anything"}"#,
            r#"{"version":1,"action":"begin_fill","origin":"https://example.test","id":"synthetic","request_id":"0123456789abcdef0123456789abcdef","owner_hwnd":1234}"#,
        ] {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, body.as_bytes()).unwrap();
            assert!(matches!(
                handle(Some(EXTENSION_ORIGIN), &mut bytes.as_slice()).status,
                Status::Error
            ));
        }
    }
}
