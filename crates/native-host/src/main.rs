//! Chrome/Brave Native Messaging host. Never opens or decrypts a vault.
use fakkarni_browser_bridge::{
    read_frame, request_desktop, write_frame, Request, Response, Status, MAX_NATIVE_REQUEST_BYTES,
};
use std::io;

const EXTENSION_ORIGIN: &str = "chrome-extension://kieggakkiiegglikoiojkbfabpmmahep/";

fn error_response() -> Response {
    Response {
        version: 1,
        status: Status::Error,
        matches: Vec::new(),
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
    let Request::Match { version, origin } = &request;
    if *version != 1 || fakkarni_vault::validate_origin(origin).is_err() {
        return error_response();
    }
    request_desktop(&request).unwrap_or_else(|_| Response {
        version: 1,
        status: Status::Unavailable,
        matches: Vec::new(),
    })
}

fn main() {
    let caller = std::env::args().nth(1);
    let response = handle(caller.as_deref(), &mut io::stdin().lock());
    // Stdout is exclusively the browser's framed JSON protocol. No credential logging.
    if let Ok(bytes) = serde_json::to_vec(&response) {
        let _ = write_frame(&mut io::stdout().lock(), &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
