//! Chrome/Brave Native Messaging host. Never opens or decrypts a vault.
use fakkarni_browser_bridge::{
    read_frame, request_desktop, write_frame, Request, Response, Status, MAX_NATIVE_REQUEST_BYTES,
};
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
    let first = request_desktop(&request);
    if first.is_ok() || !matches!(request, Request::Match { .. } | Request::BeginFill { .. }) {
        return first.unwrap_or_else(|_| Response::empty(Status::Unavailable));
    }
    if launch_desktop().is_err() {
        return Response::empty(Status::Unavailable);
    }
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(response) = request_desktop(&request) {
            return response;
        }
    }
    Response::empty(Status::Unavailable)
}

fn launch_desktop() -> io::Result<()> {
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
    command.spawn()?;
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
