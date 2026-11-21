use crate::SharedState;
use fakkarni_browser_bridge::{Credential, Status};
use fakkarni_vault::{EntryInput, EntrySummary};
use serde::Serialize;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
pub(super) struct SaveInfo {
    pub(super) token: String,
    origin: String,
    username: String,
    accounts: Option<Vec<EntrySummary>>,
}

pub(super) struct PendingSave {
    pub(super) info: SaveInfo,
    credential: Credential,
    epoch: u64,
    stamp: [u8; 32],
    change_form: bool,
    created: Instant,
}

pub(super) fn stage(
    vault: &SharedState,
    origin: String,
    credential: Credential,
    change_form: bool,
    insert: impl FnOnce(PendingSave) -> Result<(), Status>,
) -> Status {
    let epoch = vault.epoch.load(Ordering::SeqCst);
    let Ok(mut guard) = vault.inner.try_lock() else {
        return Status::Unavailable;
    };
    if vault.blocked() || guard.epoch != epoch {
        return Status::Locked;
    }
    let status = guard.vault.status();
    let (stamp, accounts) = if status.unlocked {
        match guard
            .vault
            .browser_save_options(&origin, &credential.username)
        {
            Ok((stamp, accounts)) => (stamp, Some(accounts)),
            Err(_) => return Status::Error,
        }
    } else if status.exists {
        match guard.vault.recovery_stamp() {
            Ok(stamp) => (stamp, None),
            Err(_) => return Status::Error,
        }
    } else {
        return Status::Error;
    };
    if change_form && accounts.as_ref().is_some_and(Vec::is_empty) {
        return Status::Error;
    }
    let proposal = PendingSave {
        info: SaveInfo {
            token: uuid::Uuid::new_v4().to_string(),
            origin,
            username: credential.username.clone(),
            accounts,
        },
        epoch,
        stamp,
        change_form,
        credential,
        created: Instant::now(),
    };
    insert(proposal).err().unwrap_or(Status::Pending)
}

impl PendingSave {
    pub(super) fn awaiting_unlock(&self) -> bool {
        self.info.accounts.is_none()
    }

    pub(super) fn prepare(&mut self, vault: &mut fakkarni_vault::Vault) -> Result<(), String> {
        if self.awaiting_unlock() && vault.status().unlocked {
            let (stamp, accounts) = vault
                .browser_save_options(&self.info.origin, &self.credential.username)
                .map_err(|e| e.to_string())?;
            if stamp != self.stamp {
                return Err("The vault changed. Submit the login again to retry.".into());
            }
            if self.change_form && accounts.is_empty() {
                return Err("No saved account matches this password change.".into());
            }
            self.info.accounts = Some(accounts);
        }
        Ok(())
    }

    pub(super) fn expired(&self) -> bool {
        self.created.elapsed() >= Duration::from_secs(90)
    }
    pub(super) fn current(&self, vault: &SharedState) -> bool {
        !self.expired() && !vault.blocked() && vault.epoch.load(Ordering::SeqCst) == self.epoch
    }
    pub(super) fn commit(self, vault: &SharedState, id: Option<String>) -> Result<(), String> {
        let mut guard = vault.inner.lock().map_err(|_| "Vault unavailable.")?;
        if !self.current(vault) || guard.epoch != self.epoch {
            return Err("The request expired or the vault locked. Try again.".into());
        }
        let (stamp, _) = guard
            .vault
            .browser_save_options(&self.info.origin, &self.credential.username)
            .map_err(|e| e.to_string())?;
        if stamp != self.stamp {
            return Err("The vault changed. Review a new request before saving.".into());
        }
        let accounts = self
            .info
            .accounts
            .as_ref()
            .ok_or("Unlock and review the login before saving.")?;
        let mut input = match id {
            Some(id) if accounts.iter().any(|account| account.id == id) => {
                guard.vault.edit(&id).map_err(|e| e.to_string())?
            }
            None if accounts.is_empty() => EntryInput {
                id: None,
                title: self.info.origin.chars().take(256).collect(),
                username: self.credential.username.clone(),
                url: self.info.origin.clone(),
                password: String::new(),
                notes: String::new(),
            },
            _ => return Err("Choose the exact saved account to update.".into()),
        };
        use zeroize::Zeroize;
        input.password.zeroize();
        input.password.clone_from(&self.credential.password);
        guard
            .vault
            .save_entry_when(&input, true, || self.current(vault))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GuardedVault;
    use std::sync::{
        atomic::{AtomicBool, AtomicU64},
        Mutex,
    };

    #[test]
    fn locked_proposal_waits_for_unlock_and_explicit_review() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fakkarni_vault::Vault::new(dir.path().join("locked.kdbx"));
        let master = "synthetic master for locked save";
        vault.create(master).unwrap();
        vault.lock();
        let state = SharedState {
            inner: Mutex::new(GuardedVault { vault, epoch: 0 }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        };
        let propose = || {
            let mut pending = None;
            assert_eq!(
                stage(
                    &state,
                    "https://example.test".into(),
                    Credential {
                        username: "alice".into(),
                        password: "synthetic site password".into(),
                    },
                    false,
                    |p| {
                        pending = Some(p);
                        Ok(())
                    }
                ),
                Status::Pending,
            );
            pending.unwrap()
        };
        let mut pending = propose();
        assert!(pending.awaiting_unlock());
        assert!(propose().commit(&state, None).is_err());
        {
            let mut guard = state.inner.lock().unwrap();
            guard.vault.unlock(master).unwrap();
            assert!(guard.vault.list().unwrap().is_empty());
            pending.prepare(&mut guard.vault).unwrap();
            assert!(!pending.awaiting_unlock());
            assert!(guard.vault.list().unwrap().is_empty());
        }
        pending.commit(&state, None).unwrap();
        let mut guard = state.inner.lock().unwrap();
        let entries = guard.vault.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            guard.vault.reveal(&entries[0].id).unwrap(),
            "synthetic site password"
        );
    }

    #[test]
    fn proposal_cannot_write_without_explicit_account_selection_or_after_lock() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fakkarni_vault::Vault::new(dir.path().join("save-test.kdbx"));
        vault.create("synthetic browser save master").unwrap();
        let id = vault
            .save_entry(&EntryInput {
                id: None,
                title: "Preserved title".into(),
                username: "alice".into(),
                url: "https://example.test/login".into(),
                password: "old-site-password".into(),
                notes: "Preserved note".into(),
            })
            .unwrap();
        let state = SharedState {
            inner: Mutex::new(GuardedVault { vault, epoch: 0 }),
            epoch: AtomicU64::new(0),
            os_locked: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        };
        let propose = |change_form, username: &str| {
            let mut pending = None;
            assert_eq!(
                stage(
                    &state,
                    "https://example.test".into(),
                    Credential {
                        username: username.into(),
                        password: "new-site-password".into()
                    },
                    change_form,
                    |p| {
                        pending = Some(p);
                        Ok(())
                    }
                ),
                Status::Pending
            );
            pending.unwrap()
        };
        let pending = propose(true, "");
        let info = serde_json::to_string(&pending.info).unwrap();
        assert!(!info.contains("new-site-password"));
        assert_eq!(
            state.inner.lock().unwrap().vault.reveal(&id).unwrap(),
            "old-site-password"
        );
        assert!(pending.commit(&state, None).is_err());
        assert!(propose(false, "alice")
            .commit(&state, Some(uuid::Uuid::new_v4().to_string()))
            .is_err());
        propose(false, "alice")
            .commit(&state, Some(id.clone()))
            .unwrap();
        let mut guard = state.inner.lock().unwrap();
        let entry = guard.vault.edit(&id).unwrap();
        assert_eq!(entry.password, "new-site-password");
        assert_eq!(entry.notes, "Preserved note");
        assert_eq!(entry.title, "Preserved title");
        drop(guard);
        propose(false, "bob").commit(&state, None).unwrap();
        let pending = propose(true, "");
        state.revoke();
        assert!(pending.commit(&state, Some(id)).is_err());
    }
}
