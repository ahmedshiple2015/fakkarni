use crate::{Shared, SharedState};
use fakkarni_browser_bridge::{Credential, Match, Request, Response, Status};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{Emitter, State};

const MAX_INDEX_BYTES: u64 = 512 * 1024;
const MAX_INDEX_ENTRIES: usize = 512;
const LIFETIME: Duration = Duration::from_secs(120);
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserSettings {
    pub show_accounts_when_locked: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexEntry {
    id: String,
    title: String,
    username: String,
    origin: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    settings: BrowserSettings,
    entries: Vec<IndexEntry>,
}
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
}
struct Inner {
    index: Index,
    pending: Option<Pending>,
}
pub struct BrowserState {
    inner: Mutex<Inner>,
    path: PathBuf,
}
impl BrowserState {
    pub fn new(path: PathBuf) -> Self {
        let index = read_index(&path).unwrap_or_default();
        Self {
            inner: Mutex::new(Inner {
                index,
                pending: None,
            }),
            path,
        }
    }
    fn expire(inner: &mut Inner) {
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
fn read_index(path: &std::path::Path) -> Option<Index> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return None;
    }
    let index: Index = serde_json::from_slice(&bytes).ok()?;
    if index.entries.len() > MAX_INDEX_ENTRIES
        || (!index.settings.show_accounts_when_locked && !index.entries.is_empty())
        || index.entries.iter().any(|e| {
            e.id.is_empty()
                || e.id.len() > 128
                || e.title.len() > 1024
                || e.username.len() > 2048
                || fakkarni_vault::validate_origin(&e.origin).is_err()
        })
    {
        return None;
    }
    Some(index)
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
                return Response::empty(
                    if p.info.request_id == request_id
                        && p.info.origin == origin
                        && p.entry_id == id
                    {
                        Status::Pending
                    } else {
                        Status::Unavailable
                    },
                );
            }
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
            });
            drop(inner);
            if let Some(app) = app {
                crate::lifecycle::show_main(app);
                let _ = app.emit("browser-fill-request", ());
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
#[tauri::command]
pub fn approve_browser_fill(
    state: State<'_, Arc<BrowserState>>,
    vault: State<'_, Shared>,
    request_id: String,
) -> Result<(), String> {
    approve(&state, &vault, &request_id)
}
fn approve(state: &BrowserState, vault: &SharedState, request_id: &str) -> Result<(), String> {
    let (id, origin) = {
        let mut inner = state
            .inner
            .lock()
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
    let credential = crate::run_operation(vault, epoch, |v| {
        let entry = v
            .matches(&origin)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|e| e.id == id)
            .ok_or("The account no longer matches this site.")?;
        let password = v.reveal(&id).map_err(|e| e.to_string())?;
        Ok(Credential {
            username: entry.username,
            password,
        })
    })?;
    let mut inner = state
        .inner
        .lock()
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
    fn fill_needs_approval_exact_origin_and_is_single_use() {
        let (_dir, vault, state, id) = fixture();
        let token = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            begin(&vault, &state, "wrong-id", token).status,
            Status::Error
        );
        assert_eq!(begin(&vault, &state, &id, token).status, Status::Pending);
        assert_eq!(
            poll(&vault, &state, "https://example.test", token).status,
            Status::Pending
        );
        assert!(poll(&vault, &state, "https://evil.test", token)
            .credential
            .is_none());
        approve(&state, &vault, token).unwrap();
        assert!(poll(&vault, &state, "https://evil.test", token)
            .credential
            .is_none());
        let response = poll(&vault, &state, "https://example.test", token);
        assert_eq!(response.credential.unwrap().password, "synthetic-secret");
        assert_eq!(
            poll(&vault, &state, "https://example.test", token).status,
            Status::Cancelled
        );
    }
    #[test]
    fn lock_cancel_and_expiry_revoke_approved_delivery() {
        let (_dir, vault, state, id) = fixture();
        let token = "0123456789abcdef0123456789abcdef";
        begin(&vault, &state, &id, token);
        approve(&state, &vault, token).unwrap();
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
        assert!(approve(&state, &vault, token).is_err());
        begin(&vault, &state, &id, token);
        state
            .inner
            .lock()
            .unwrap()
            .pending
            .as_mut()
            .unwrap()
            .started = Instant::now() - LIFETIME;
        assert!(approve(&state, &vault, token).is_err());
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
        assert!(approve(&state, &vault, token).is_err());
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
}
