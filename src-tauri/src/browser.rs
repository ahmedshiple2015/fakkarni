use crate::{Shared, SharedState};
use fakkarni_browser_bridge::{Credential, Match, Request, Response, Status};
pub use fakkarni_vault::account_index::BrowserSettings;
use fakkarni_vault::account_index::{
    read_index, Index, IndexEntry, MAX_INDEX_BYTES, MAX_INDEX_ENTRIES,
};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::State;

const LIFETIME: Duration = Duration::from_secs(120);
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInfo {
    request_id: String,
    origin: String,
    title: String,
    username: String,
}
struct Pending {
    info: PendingInfo,
    entry_id: String,
    started: Instant,
    ready: Option<(Credential, u64)>,
    unlock_started: bool,
}
struct Inner {
    index: Index,
    pending: Option<Pending>,
    used_request_ids: Vec<(String, Instant)>,
}
pub struct BrowserState {
    inner: Mutex<Inner>,
    path: PathBuf,
}
impl BrowserState {
    pub(super) fn claim_unlock(&self, request_id: &str) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        Self::expire(&mut inner);
        let Some(pending) = inner
            .pending
            .as_mut()
            .filter(|p| p.info.request_id == request_id && p.ready.is_none() && !p.unlock_started)
        else {
            return false;
        };
        pending.unlock_started = true;
        true
    }
    pub(super) fn is_waiting(&self, request_id: &str) -> bool {
        // Polling holds this briefly and only ever try-locks the vault. Waiting
        // here avoids rejecting valid PIN results on harmless polling contention.
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        Self::expire(&mut inner);
        inner
            .pending
            .as_ref()
            .is_some_and(|p| p.info.request_id == request_id && p.ready.is_none())
    }
    pub(super) fn cancel_waiting(&self, request_id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            Self::expire(&mut inner);
            if inner
                .pending
                .as_ref()
                .is_some_and(|p| p.info.request_id == request_id && p.ready.is_none())
            {
                inner.pending = None;
            }
        }
    }
    pub fn new(path: PathBuf) -> Self {
        let index = read_index(&path).unwrap_or_default();
        Self {
            inner: Mutex::new(Inner {
                index,
                pending: None,
                used_request_ids: Vec::new(),
            }),
            path,
        }
    }
    fn expire(inner: &mut Inner) {
        inner
            .used_request_ids
            .retain(|(_, started)| started.elapsed() < LIFETIME);
        if inner
            .pending
            .as_ref()
            .is_some_and(|p| p.started.elapsed() >= LIFETIME)
        {
            inner.pending = None;
        }
    }
    pub fn revoke_ready(&self) {
        if let Ok(mut inner) = self.inner.try_lock() {
            Self::expire(&mut inner);
            if inner.pending.as_ref().is_some_and(|p| p.ready.is_some()) {
                inner.pending = None;
            }
        }
    }
    /// A replacement vault must not inherit account metadata or pending fills.
    pub fn reset_after_restore(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Browser index could not be reset.")?;
        inner.pending = None;
        inner.used_request_ids.clear();
        inner.index = Index::default();
        let bytes =
            serde_json::to_vec(&inner.index).map_err(|_| "Browser index could not be reset.")?;
        if self.persist(&bytes).is_err() {
            match fs::remove_file(&self.path) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return Err("The vault was restored, but the old browser index file could not be removed. Locked-account suggestions are disabled in this session; check local file permissions before restarting.".into()),
            }
        }
        Ok(())
    }
    pub fn forget_entry(&self, id: &str) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "The login was deleted, but browser metadata cleanup failed.")?;
        if inner
            .pending
            .as_ref()
            .is_some_and(|pending| pending.entry_id == id)
        {
            inner.pending = None;
        }
        inner.index.entries.retain(|entry| entry.id != id);
        let bytes =
            serde_json::to_vec(&inner.index).map_err(|_| "Browser metadata cleanup failed.")?;
        if self.persist(&bytes).is_err() {
            // Fail closed if we cannot update the metadata cache: disable the
            // index and clear its file rather than retain a deleted account.
            drop(inner);
            self.reset_after_restore().map_err(|_| "The login was deleted, but its browser cache could not be removed. Suggestions are disabled in this session; check file permissions before restarting.".to_owned())?;
            return Err("The login was deleted. Locked account suggestions were reset because their cache could not be updated.".into());
        }
        Ok(())
    }
    /// The extension account click authorizes this exact pending request. Unlocking
    /// resumes it automatically; no additional desktop approval prompt is shown.
    pub fn complete_after_unlock(&self, vault: &SharedState) {
        let request_id = {
            let Ok(mut inner) = self.inner.try_lock() else {
                return;
            };
            Self::expire(&mut inner);
            inner
                .pending
                .as_ref()
                .filter(|p| p.ready.is_none())
                .map(|p| p.info.request_id.clone())
        };
        if let Some(request_id) = request_id {
            let _ = prepare(self, vault, &request_id);
        }
    }
    pub fn refresh(&self, vault: &mut fakkarni_vault::Vault) {
        let Ok(mut inner) = self.inner.try_lock() else {
            return;
        };
        if !inner.index.settings.show_accounts_when_locked {
            return;
        }
        let Ok(entries) = vault.browser_index() else {
            return;
        };
        let entries = entries
            .into_iter()
            .filter_map(|e| {
                let parsed = url::Url::parse(&e.url).ok()?;
                if parsed.scheme() != "https"
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return None;
                }
                let origin = parsed.origin().ascii_serialization();
                if fakkarni_vault::validate_origin(&origin).is_err() {
                    return None;
                }
                Some(IndexEntry {
                    id: e.id,
                    title: e.title.chars().take(256).collect(),
                    username: e.username.chars().take(512).collect(),
                    origin,
                })
            })
            .take(MAX_INDEX_ENTRIES)
            .collect();
        let next = Index {
            settings: inner.index.settings.clone(),
            entries,
        };
        let Ok(bytes) = serde_json::to_vec(&next) else {
            return;
        };
        if bytes.len() as u64 > MAX_INDEX_BYTES {
            return;
        }
        if serde_json::to_vec(&inner.index).ok().as_deref() != Some(bytes.as_slice())
            && self.persist(&bytes).is_ok()
        {
            inner.index = next;
        }
    }
    fn persist(&self, bytes: &[u8]) -> Result<(), String> {
        let parent = self.path.parent().ok_or("Invalid browser settings path.")?;
        fs::create_dir_all(parent).map_err(|_| "Couldn't save browser preferences.")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "Couldn't save browser preferences.")?;
        temp.write_all(bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|_| "Couldn't save browser preferences.")?;
        temp.persist(&self.path)
            .map_err(|_| "Couldn't save browser preferences.")?;
        Ok(())
    }
}
fn matches(vault: &SharedState, state: &BrowserState, origin: &str) -> Response {
    let epoch = vault.epoch.load(Ordering::SeqCst);
    if vault.blocked() {
        return Response::empty(Status::Locked);
    }
    let Ok(mut guard) = vault.inner.try_lock() else {
        return Response::empty(Status::Unavailable);
    };
    if guard.epoch != epoch {
        guard.vault.lock();
        guard.epoch = epoch;
    }
    let (status, entries) = match guard.vault.matches(origin) {
        Ok(entries) => (
            Status::Ok,
            entries
                .into_iter()
                .map(|e| Match {
                    id: e.id,
                    title: e.title,
                    username: e.username,
                    url: origin.into(),
                })
                .collect(),
        ),
        Err(fakkarni_vault::VaultError::Locked) => {
            let Ok(inner) = state.inner.try_lock() else {
                return Response::empty(Status::Unavailable);
            };
            let entries = if inner.index.settings.show_accounts_when_locked {
                inner
                    .index
                    .entries
                    .iter()
                    .filter(|e| e.origin == origin)
                    .take(50)
                    .map(|e| Match {
                        id: e.id.clone(),
                        title: e.title.clone(),
                        username: e.username.clone(),
                        url: origin.into(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            (Status::Locked, entries)
        }
        Err(_) => return Response::empty(Status::Error),
    };
    if vault.blocked() || vault.epoch.load(Ordering::SeqCst) != epoch {
        return Response::empty(Status::Locked);
    }
    Response {
        version: 1,
        status,
        matches: entries,
        credential: None,
    }
}
fn focus_if_waiting(state: &BrowserState, app: Option<&tauri::AppHandle>) {
    let request_id = state.inner.try_lock().ok().and_then(|inner| {
        inner
            .pending
            .as_ref()
            .filter(|p| p.ready.is_none())
            .map(|p| p.info.request_id.clone())
    });
    if let (Some(request_id), Some(app)) = (request_id, app) {
        crate::browser_unlock::begin(app, request_id);
    }
}

pub fn handle(
    vault: &SharedState,
    state: &BrowserState,
    app: Option<&tauri::AppHandle>,
    request: Request,
) -> Response {
    if request.validate().is_err() || fakkarni_vault::validate_origin(request.origin()).is_err() {
        return Response::empty(Status::Error);
    }
    match request {
        Request::Match { origin, .. } => matches(vault, state, &origin),
        Request::BeginFill {
            origin,
            id,
            request_id,
            ..
        } => {
            let response = matches(vault, state, &origin);
            let Some(entry) = response.matches.into_iter().find(|e| e.id == id) else {
                return Response::empty(if response.status == Status::Locked {
                    Status::Locked
                } else {
                    Status::Error
                });
            };
            let Ok(mut inner) = state.inner.try_lock() else {
                return Response::empty(Status::Unavailable);
            };
            BrowserState::expire(&mut inner);
            if let Some(p) = inner.pending.as_ref() {
                let same =
                    p.info.request_id == request_id && p.info.origin == origin && p.entry_id == id;
                let waiting = p.ready.is_none();
                drop(inner);
                if same && waiting {
                    state.complete_after_unlock(vault);
                    if response.status == Status::Locked {
                        focus_if_waiting(state, app);
                    }
                }
                return Response::empty(if same {
                    Status::Pending
                } else {
                    Status::Unavailable
                });
            }
            if inner
                .used_request_ids
                .iter()
                .any(|(used, _)| used == &request_id)
            {
                return Response::empty(Status::Cancelled);
            }
            if inner.used_request_ids.len() >= 128 {
                return Response::empty(Status::Unavailable);
            }
            inner
                .used_request_ids
                .push((request_id.clone(), Instant::now()));
            inner.pending = Some(Pending {
                info: PendingInfo {
                    request_id,
                    origin,
                    title: entry.title,
                    username: entry.username,
                },
                entry_id: id,
                started: Instant::now(),
                ready: None,
                unlock_started: false,
            });
            drop(inner);
            state.complete_after_unlock(vault);
            if response.status == Status::Locked {
                focus_if_waiting(state, app);
            }
            Response::empty(Status::Pending)
        }
        Request::CancelFill {
            origin, request_id, ..
        } => {
            let Ok(mut inner) = state.inner.try_lock() else {
                return Response::empty(Status::Unavailable);
            };
            BrowserState::expire(&mut inner);
            if inner
                .pending
                .as_ref()
                .is_some_and(|p| p.info.request_id == request_id && p.info.origin == origin)
            {
                inner.pending = None;
            }
            Response::empty(Status::Cancelled)
        }
        Request::PollFill {
            origin, request_id, ..
        } => {
            let Ok(mut inner) = state.inner.try_lock() else {
                return Response::empty(Status::Unavailable);
            };
            BrowserState::expire(&mut inner);
            let Some(p) = inner.pending.as_ref() else {
                return Response::empty(Status::Cancelled);
            };
            if p.info.request_id != request_id || p.info.origin != origin {
                return Response::empty(Status::Error);
            }
            if let Some((_, epoch)) = p.ready.as_ref() {
                if vault.blocked() || *epoch != vault.epoch.load(Ordering::SeqCst) {
                    inner.pending = None;
                    return Response::empty(Status::Cancelled);
                }
                // Expire the underlying vault too: idle expiration must revoke approved data.
                let Ok(mut guard) = vault.inner.try_lock() else {
                    return Response::empty(Status::Unavailable);
                };
                if !guard.vault.status().unlocked
                    || guard.epoch != *epoch
                    || vault.blocked()
                    || vault.epoch.load(Ordering::SeqCst) != *epoch
                {
                    inner.pending = None;
                    return Response::empty(Status::Cancelled);
                }
                let mut p = inner.pending.take().expect("checked pending");
                let (credential, _) = p.ready.take().expect("checked ready");
                let mut response = Response::empty(Status::Ok);
                response.credential = Some(credential);
                response
            } else {
                Response::empty(Status::Pending)
            }
        }
    }
}
#[tauri::command]
pub fn browser_settings(state: State<'_, Arc<BrowserState>>) -> Result<BrowserSettings, String> {
    Ok(state
        .inner()
        .inner
        .lock()
        .map_err(|_| "Browser preferences unavailable.")?
        .index
        .settings
        .clone())
}
#[tauri::command]
pub fn set_browser_settings(
    state: State<'_, Arc<BrowserState>>,
    vault: State<'_, Shared>,
    settings: BrowserSettings,
) -> Result<BrowserSettings, String> {
    let epoch = vault.epoch.load(Ordering::SeqCst);
    crate::run_operation(&vault, epoch, |v| {
        v.identity().map_err(|e| e.to_string())?;
        Ok(())
    })?;
    let mut inner = state
        .inner()
        .inner
        .lock()
        .map_err(|_| "Browser preferences unavailable.")?;
    let next = Index {
        settings: settings.clone(),
        entries: Vec::new(),
    };
    let bytes = serde_json::to_vec(&next).map_err(|_| "Browser preferences unavailable.")?;
    state.persist(&bytes)?;
    inner.index = next;
    drop(inner);
    if settings.show_accounts_when_locked {
        crate::run_operation(&vault, epoch, |v| {
            state.refresh(v);
            Ok(())
        })?;
    }
    Ok(settings)
}
#[tauri::command]
pub fn browser_pending(state: State<'_, Arc<BrowserState>>) -> Result<Option<PendingInfo>, String> {
    let mut inner = state
        .inner()
        .inner
        .lock()
        .map_err(|_| "Browser request unavailable.")?;
    BrowserState::expire(&mut inner);
    Ok(inner
        .pending
        .as_ref()
        .filter(|p| p.ready.is_none())
        .map(|p| p.info.clone()))
}
#[tauri::command]
pub fn reject_browser_fill(
    state: State<'_, Arc<BrowserState>>,
    request_id: String,
) -> Result<(), String> {
    let mut inner = state
        .inner()
        .inner
        .lock()
        .map_err(|_| "Browser request unavailable.")?;
    if inner
        .pending
        .as_ref()
        .is_some_and(|p| p.info.request_id == request_id)
    {
        inner.pending = None;
    }
    Ok(())
}
fn prepare(state: &BrowserState, vault: &SharedState, request_id: &str) -> Result<(), String> {
    let (id, origin) = {
        let mut inner = state
            .inner
            .try_lock()
            .map_err(|_| "Browser request unavailable.")?;
        BrowserState::expire(&mut inner);
        let p = inner
            .pending
            .as_ref()
            .filter(|p| p.info.request_id == request_id && p.ready.is_none())
            .ok_or("Browser request expired.")?;
        (p.entry_id.clone(), p.info.origin.clone())
    };
    let epoch = vault.epoch.load(Ordering::SeqCst);
    if vault.blocked() {
        return Err("Windows is locked.".into());
    }
    let mut guard = vault.inner.try_lock().map_err(|_| "Vault is busy.")?;
    if guard.epoch != epoch {
        guard.vault.lock();
        guard.epoch = epoch;
    }
    let entry = guard
        .vault
        .matches(&origin)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or("The account no longer matches this site.")?;
    let credential = Credential {
        username: entry.username,
        password: guard.vault.reveal(&id).map_err(|e| e.to_string())?,
    };
    let mut inner = state
        .inner
        .try_lock()
        .map_err(|_| "Browser request unavailable.")?;
    BrowserState::expire(&mut inner);
    if vault.blocked() || vault.epoch.load(Ordering::SeqCst) != epoch {
        return Err("Vault locked. Retry filling.".into());
    }
    let p = inner
        .pending
        .as_mut()
        .filter(|p| {
            p.info.request_id == request_id
                && p.info.origin == origin
                && p.entry_id == id
                && p.ready.is_none()
        })
        .ok_or("Browser request expired.")?;
    p.ready = Some((credential, epoch));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GuardedVault;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    fn fixture() -> (tempfile::TempDir, SharedState, BrowserState, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fakkarni_vault::Vault::new(dir.path().join("test.kdbx"));
        vault.create("Synthetic test master password").unwrap();
        let id = vault
            .save_entry(&fakkarni_vault::EntryInput {
                id: None,
                title: "Account".into(),
                username: "synthetic-user".into(),
                password: "synthetic-secret".into(),
                url: "https://example.test/login?private=query".into(),
                notes: "private-note".into(),
            })
            .unwrap();
        let shared = SharedState {
            inner: Mutex::new(GuardedVault { vault, epoch: 0 }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        };
        let state = BrowserState::new(dir.path().join("index.json"));
        (dir, shared, state, id)
    }
    fn begin(vault: &SharedState, state: &BrowserState, id: &str, token: &str) -> Response {
        handle(
            vault,
            state,
            None,
            Request::BeginFill {
                version: 1,
                origin: "https://example.test".into(),
                id: id.into(),
                request_id: token.into(),
            },
        )
    }
    fn poll(vault: &SharedState, state: &BrowserState, origin: &str, token: &str) -> Response {
        handle(
            vault,
            state,
            None,
            Request::PollFill {
                version: 1,
                origin: origin.into(),
                request_id: token.into(),
            },
        )
    }
    #[test]
    fn pin_prompt_claim_is_single_use_and_cancel_or_expiry_revokes_it() {
        let (_dir, vault, state, id) = fixture();
        state
            .inner
            .lock()
            .unwrap()
            .index
            .settings
            .show_accounts_when_locked = true;
        state.refresh(&mut vault.inner.lock().unwrap().vault);
        vault.inner.lock().unwrap().vault.lock();
        let first = "a".repeat(32);
        assert_eq!(begin(&vault, &state, &id, &first).status, Status::Pending);
        assert!(!state.claim_unlock("wrong-request"));
        assert!(state.claim_unlock(&first));
        assert!(!state.claim_unlock(&first));
        assert!(state.is_waiting(&first));
        state.cancel_waiting(&first);
        assert!(!state.is_waiting(&first));
        assert!(!state.claim_unlock(&first));
        assert_eq!(begin(&vault, &state, &id, &first).status, Status::Cancelled);
        let second = "b".repeat(32);
        assert_eq!(begin(&vault, &state, &id, &second).status, Status::Pending);
        state.cancel_waiting(&first);
        assert!(state.is_waiting(&second));
        state
            .inner
            .lock()
            .unwrap()
            .pending
            .as_mut()
            .unwrap()
            .started = Instant::now() - LIFETIME;
        assert!(!state.claim_unlock(&second));
        assert!(!state.is_waiting(&second));
    }
    #[test]
    fn repeated_wire_fills_and_credential_drops_do_not_modify_stored_password() {
        let (dir, vault, mut state, id) = fixture();
        let path = dir.path().join("test.kdbx");
        let original_disk = fs::read(&path).unwrap();
        state
            .inner
            .lock()
            .unwrap()
            .index
            .settings
            .show_accounts_when_locked = true;
        state.refresh(&mut vault.inner.lock().unwrap().vault);
        for restart in 0..4 {
            for fill in 0..10 {
                let token = format!("{:032x}", restart * 10 + fill);
                let suggestions = handle(
                    &vault,
                    &state,
                    None,
                    Request::Match {
                        version: 1,
                        origin: "https://example.test".into(),
                    },
                );
                assert_eq!(suggestions.matches[0].username, "synthetic-user");
                assert!(suggestions.credential.is_none());
                assert_eq!(begin(&vault, &state, &id, &token).status, Status::Pending);
                let response = poll(&vault, &state, "https://example.test", &token);
                assert_eq!(response.status, Status::Ok);
                let encoded = zeroize::Zeroizing::new(serde_json::to_vec(&response).unwrap());
                let decoded: Response = serde_json::from_slice(&encoded).unwrap();
                let credential = decoded.credential.as_ref().unwrap();
                assert_eq!(credential.username, "synthetic-user");
                assert_eq!(credential.password, "synthetic-secret");
                assert_ne!(credential.username, credential.password);
                // Exercise all secret-buffer consumers, including independent clone
                // drops and the original response's Credential::drop implementation.
                let cloned = credential.clone();
                drop(cloned);
                drop(decoded);
                drop(response);
                drop(encoded);
                assert_eq!(
                    poll(&vault, &state, "https://example.test", &token).status,
                    Status::Cancelled
                );
                let mut guard = vault.inner.lock().unwrap();
                assert_eq!(guard.vault.reveal(&id).unwrap(), "synthetic-secret");
                let input = guard.vault.edit(&id).unwrap();
                assert_eq!(input.username, "synthetic-user");
                assert_eq!(input.password, "synthetic-secret");
                assert_eq!(input.notes, "private-note");
                assert_eq!(fs::read(&path).unwrap(), original_disk);
            }
            vault.revoke();
            {
                let mut guard = vault.inner.lock().unwrap();
                // Simulate an application update/restart by constructing a fresh
                // reader of the existing encrypted file, without saving anything.
                guard.vault = fakkarni_vault::Vault::new(path.clone());
                guard
                    .vault
                    .unlock("Synthetic test master password")
                    .unwrap();
                guard
                    .vault
                    .verify_password("Synthetic test master password")
                    .unwrap();
                assert_eq!(guard.vault.reveal(&id).unwrap(), "synthetic-secret");
            }
            state = BrowserState::new(dir.path().join("index.json"));
            assert_eq!(fs::read(&path).unwrap(), original_disk);
        }
    }
    #[test]
    fn unlocked_fill_is_automatic_exact_origin_single_use_and_replay_safe() {
        let (_dir, vault, state, id) = fixture();
        let token = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            begin(&vault, &state, "wrong-id", token).status,
            Status::Error
        );
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Pending);
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Pending);
        assert!(poll(&vault, &state, "https://evil.test", token)
            .credential
            .is_none());
        assert!(poll(&vault, &state, "https://evil.test", token)
            .credential
            .is_none());
        let response = poll(&vault, &state, "https://example.test", token);
        assert_eq!(response.credential.unwrap().password, "synthetic-secret");
        assert_eq!(
            poll(&vault, &state, "https://example.test", token).status,
            Status::Cancelled
        );
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Cancelled);
    }
    #[test]
    fn lock_cancel_and_expiry_revoke_automatic_delivery() {
        let (_dir, vault, state, id) = fixture();
        let token = "0123456789abcdef0123456789abcdef";
        begin(&vault, &state, &id, token);
        vault.revoke();
        assert!(poll(&vault, &state, "https://example.test", token)
            .credential
            .is_none());
        assert!(state.inner.lock().unwrap().pending.is_none());
        vault
            .inner
            .lock()
            .unwrap()
            .vault
            .unlock("Synthetic test master password")
            .unwrap();
        let token = "1123456789abcdef0123456789abcdef";
        begin(&vault, &state, &id, token);
        handle(
            &vault,
            &state,
            None,
            Request::CancelFill {
                version: 1,
                origin: "https://example.test".into(),
                request_id: token.into(),
            },
        );
        state.complete_after_unlock(&vault);
        assert!(poll(&vault, &state, "https://example.test", token)
            .credential
            .is_none());
        let token = "2123456789abcdef0123456789abcdef";
        begin(&vault, &state, &id, token);
        state
            .inner
            .lock()
            .unwrap()
            .pending
            .as_mut()
            .unwrap()
            .started = Instant::now() - LIFETIME;
        state.complete_after_unlock(&vault);
        assert!(poll(&vault, &state, "https://example.test", token)
            .credential
            .is_none());
    }
    #[test]
    fn locked_index_opt_in_stores_only_bounded_account_metadata() {
        let (dir, vault, state, id) = fixture();
        let token = "0123456789abcdef0123456789abcdef";
        assert!(
            !state
                .inner
                .lock()
                .unwrap()
                .index
                .settings
                .show_accounts_when_locked
        );
        state
            .inner
            .lock()
            .unwrap()
            .index
            .settings
            .show_accounts_when_locked = true;
        state.refresh(&mut vault.inner.lock().unwrap().vault);
        let text = fs::read_to_string(dir.path().join("index.json")).unwrap();
        for private in [
            "synthetic-secret",
            "private-note",
            "private=query",
            "/login",
        ] {
            assert!(!text.contains(private));
        }
        vault.revoke();
        let matches = handle(
            &vault,
            &state,
            None,
            Request::Match {
                version: 1,
                origin: "https://example.test".into(),
            },
        );
        assert_eq!(matches.status, Status::Locked);
        assert_eq!(matches.matches.len(), 1);
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Pending);
        state.complete_after_unlock(&vault);
        assert_eq!(
            poll(&vault, &state, "https://example.test", token).status,
            Status::Pending
        );
        vault
            .inner
            .lock()
            .unwrap()
            .vault
            .unlock("Synthetic test master password")
            .unwrap();
        state.complete_after_unlock(&vault);
        assert_eq!(
            poll(&vault, &state, "https://example.test", token)
                .credential
                .unwrap()
                .password,
            "synthetic-secret"
        );
        let mut bytes = text.into_bytes();
        bytes.extend(vec![b' '; MAX_INDEX_BYTES as usize]);
        fs::write(dir.path().join("index.json"), bytes).unwrap();
        assert!(
            !BrowserState::new(dir.path().join("index.json"))
                .inner
                .lock()
                .unwrap()
                .index
                .settings
                .show_accounts_when_locked
        );
    }

    #[test]
    fn deletion_removes_cached_account_and_revokes_its_ready_fill() {
        let (dir, vault, state, id) = fixture();
        let other = vault
            .inner
            .lock()
            .unwrap()
            .vault
            .save_entry(&fakkarni_vault::EntryInput {
                id: None,
                title: "Retained account".into(),
                username: "retained-user".into(),
                password: "retained-secret".into(),
                url: "https://example.test".into(),
                notes: "".into(),
            })
            .unwrap();
        state
            .inner
            .lock()
            .unwrap()
            .index
            .settings
            .show_accounts_when_locked = true;
        state.refresh(&mut vault.inner.lock().unwrap().vault);
        let token = "0123456789abcdef0123456789abcdef";
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Pending);
        vault.inner.lock().unwrap().vault.delete_entry(&id).unwrap();
        state.forget_entry(&id).unwrap();
        assert!(poll(&vault, &state, "https://example.test", token)
            .credential
            .is_none());
        let reopened = BrowserState::new(dir.path().join("index.json"));
        let inner = reopened.inner.lock().unwrap();
        assert_eq!(inner.index.entries.len(), 1);
        assert_eq!(inner.index.entries[0].id, other);
        assert!(!fs::read_to_string(dir.path().join("index.json"))
            .unwrap()
            .contains(&id));
        assert_eq!(
            vault.inner.lock().unwrap().vault.reveal(&other).unwrap(),
            "retained-secret"
        );
    }

    #[test]
    fn restore_reset_removes_ready_fills_replay_ids_and_persisted_account_index() {
        let (dir, vault, state, id) = fixture();
        let token = "0123456789abcdef0123456789abcdef";
        state
            .inner
            .lock()
            .unwrap()
            .index
            .settings
            .show_accounts_when_locked = true;
        state.refresh(&mut vault.inner.lock().unwrap().vault);
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Pending);
        {
            let inner = state.inner.lock().unwrap();
            assert!(inner.pending.as_ref().unwrap().ready.is_some());
            assert!(!inner.used_request_ids.is_empty());
            assert_eq!(inner.index.entries.len(), 1);
        }
        state.reset_after_restore().unwrap();
        {
            let inner = state.inner.lock().unwrap();
            assert!(inner.pending.is_none());
            assert!(inner.used_request_ids.is_empty());
            assert!(inner.index.entries.is_empty());
            assert!(!inner.index.settings.show_accounts_when_locked);
        }
        assert!(poll(&vault, &state, "https://example.test", token)
            .credential
            .is_none());
        let persisted = fs::read_to_string(dir.path().join("index.json")).unwrap();
        assert!(!persisted.contains("synthetic-user"));
        assert!(!persisted.contains(&id));
        let reopened = BrowserState::new(dir.path().join("index.json"));
        let reopened = reopened.inner.lock().unwrap();
        assert!(reopened.index.entries.is_empty());
        assert!(!reopened.index.settings.show_accounts_when_locked);
        // Resetting browser metadata must never alter the vault's password.
        assert_eq!(
            vault.inner.lock().unwrap().vault.reveal(&id).unwrap(),
            "synthetic-secret"
        );
    }

    #[test]
    fn restore_index_cleanup_failure_reports_warning_but_disables_in_memory_access() {
        let (dir, vault, state, id) = fixture();
        let token = "0123456789abcdef0123456789abcdef";
        state
            .inner
            .lock()
            .unwrap()
            .index
            .settings
            .show_accounts_when_locked = true;
        state.refresh(&mut vault.inner.lock().unwrap().vault);
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Pending);
        // A directory in place of the index blocks both atomic file replacement
        // and remove_file without altering production error-handling paths.
        let index = dir.path().join("index.json");
        fs::remove_file(&index).unwrap();
        fs::create_dir(&index).unwrap();
        let warning = state.reset_after_restore().unwrap_err();
        assert!(warning.contains("vault was restored"));
        assert!(warning.contains("could not be removed"));
        let inner = state.inner.lock().unwrap();
        assert!(inner.pending.is_none());
        assert!(inner.used_request_ids.is_empty());
        assert!(inner.index.entries.is_empty());
        assert!(!inner.index.settings.show_accounts_when_locked);
    }
}
