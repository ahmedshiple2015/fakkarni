//! Temporary debug-build cold-start trace. Only static stages and HRESULT codes;
//! never record requests, account data, credentials, provider text, or PIN input.
#[cfg(debug_assertions)]
mod debug {
    use serde::Serialize;
    use std::{
        io::Write,
        sync::{Mutex, OnceLock},
        time::Instant,
    };
    #[derive(Clone, Serialize)]
    struct Event {
        elapsed_ms: u128,
        stage: &'static str,
        code: Option<u32>,
    }
    static TRACE: OnceLock<Mutex<(Instant, Vec<Event>)>> = OnceLock::new();
    pub(super) fn record(stage: &'static str, code: Option<u32>) {
        let Ok(mut state) = TRACE
            .get_or_init(|| Mutex::new((Instant::now(), Vec::new())))
            .lock()
        else {
            return;
        };
        let elapsed_ms = state.0.elapsed().as_millis();
        if state.1.len() >= 96 {
            state.1.remove(0);
        }
        state.1.push(Event {
            elapsed_ms,
            stage,
            code,
        });
        let report = serde_json::json!({"version":"cold-pin-diagnostic-v1","processId":std::process::id(),"events":state.1});
        let Some(parent) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        else {
            return;
        };
        let Ok(mut file) = tempfile::NamedTempFile::new_in(&parent) else {
            return;
        };
        if serde_json::to_writer_pretty(&mut file, &report).is_ok() && file.flush().is_ok() {
            let _ = file.persist(parent.join("fakkarni-unlock-trace.json"));
        }
    }
}
pub(super) fn record(stage: &'static str) {
    #[cfg(debug_assertions)]
    debug::record(stage, None);
    #[cfg(not(debug_assertions))]
    let _ = stage;
}
pub(super) fn failure(stage: &'static str, error: &str) {
    #[cfg(debug_assertions)]
    debug::record(stage, error_code(error));
    #[cfg(not(debug_assertions))]
    let _ = (stage, error);
}
pub(super) fn provider_failure(operation: &'static str, code: u32) {
    #[cfg(debug_assertions)]
    debug::record(
        match operation {
            "provider access" => "pin_cng_provider_error",
            "key access" => "pin_cng_key_open_error",
            "key policy setup" => "pin_cng_owner_or_policy_error",
            "key policy verification" => "pin_cng_policy_error",
            "unlock" => "pin_cng_decrypt_error",
            _ => "pin_cng_error",
        },
        Some(code),
    );
    #[cfg(not(debug_assertions))]
    let _ = (operation, code);
}
#[cfg(any(debug_assertions, test))]
fn error_code(error: &str) -> Option<u32> {
    error.match_indices("0x").find_map(|(offset, _)| {
        let code = error.get(offset + 2..offset + 10)?;
        if code.bytes().all(|b| b.is_ascii_hexdigit()) {
            u32::from_str_radix(code, 16).ok()
        } else {
            None
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_numeric_error_code_is_extracted() {
        assert_eq!(
            error_code("Synthetic private provider text (0x80090022)."),
            Some(0x80090022)
        );
        assert_eq!(error_code("Synthetic user data without a code"), None);
        assert_eq!(error_code("0xnotvalid"), None);
    }
}
