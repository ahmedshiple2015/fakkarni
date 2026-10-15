//! Local, authenticated enrollment metadata. No master password is persisted in plaintext.
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
};
use zeroize::Zeroizing;

const MAX_FILE: u64 = 16 * 1024;
const VERSION: u8 = 2;
const SCHEME: &str = "RSAES-PKCS1-v1_5+A256GCM";
const ERROR: &str =
    "Windows Hello enrollment is unavailable or invalid. Unlock with your master password.";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    scheme: String,
    key_name: String,
    vault_id: String,
    wrapped_key: Vec<u8>,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

impl Envelope {
    fn validate(&self) -> Result<(), String> {
        let id = uuid::Uuid::parse_str(&self.vault_id).map_err(|_| ERROR)?;
        if self.version != VERSION
            || self.scheme != SCHEME
            || id.hyphenated().to_string() != self.vault_id
            || !valid_key_name(&self.key_name)
            || self.wrapped_key.len() != 256
            || self.nonce.len() != 12
            || !(16..=1040).contains(&self.ciphertext.len())
        {
            return Err(ERROR.into());
        }
        Ok(())
    }
}

fn valid_key_name(name: &str) -> bool {
    let Some((sid, id)) = name.split_once("//Fakkarni//") else {
        return false;
    };
    name.len() <= 256
        && sid.starts_with("S-1-")
        && sid
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'S' || b == b'-')
        && uuid::Uuid::parse_str(id)
            .map(|u| !u.is_nil() && u.hyphenated().to_string() == id)
            .unwrap_or(false)
}

pub struct UnlockedSecret {
    pub password: Zeroizing<String>,
    pub vault_id: String,
}

trait Protector {
    fn name(&self) -> Result<String, String>;
    fn create(&self, name: &str, hwnd: isize) -> Result<(), String>;
    fn seal(&self, name: &str, plaintext: &[u8], hwnd: isize) -> Result<Vec<u8>, String>;
    fn open(
        &self,
        name: &str,
        ciphertext: &[u8],
        hwnd: isize,
    ) -> Result<Zeroizing<Vec<u8>>, String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}
struct WindowsHello;
impl Protector for WindowsHello {
    fn name(&self) -> Result<String, String> {
        fakkarni_windows_hello::new_key_name()
    }
    fn create(&self, name: &str, hwnd: isize) -> Result<(), String> {
        fakkarni_windows_hello::create(name, hwnd)
    }
    fn seal(&self, name: &str, plaintext: &[u8], hwnd: isize) -> Result<Vec<u8>, String> {
        fakkarni_windows_hello::seal(name, plaintext, hwnd)
    }
    fn open(
        &self,
        name: &str,
        ciphertext: &[u8],
        hwnd: isize,
    ) -> Result<Zeroizing<Vec<u8>>, String> {
        fakkarni_windows_hello::open(name, ciphertext, hwnd)
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        fakkarni_windows_hello::delete(name)
    }
}

pub struct Manager {
    enrollment_path: PathBuf,
    vault_path: PathBuf,
}
impl Manager {
    pub fn new(enrollment_path: PathBuf, vault_path: PathBuf) -> Self {
        Self {
            enrollment_path,
            vault_path,
        }
    }
    fn read(&self) -> Result<Option<Envelope>, String> {
        let file = match File::open(&self.enrollment_path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ERROR.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ERROR)?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(ERROR.into());
        }
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
        envelope.validate()?;
        Ok(Some(envelope))
    }
    pub fn enabled(&self) -> Result<bool, String> {
        Ok(self.read()?.is_some())
    }
    fn aad(&self, envelope: &Envelope) -> Vec<u8> {
        // Authenticate every algorithm/binding field. Re-enrollment, not an
        // automatic padding fallback, is required when the scheme changes.
        let path = self.vault_path.to_string_lossy();
        let mut result = b"Fakkarni.Hello.Enrollment\0".to_vec();
        result.push(envelope.version);
        for field in [
            envelope.scheme.as_str(),
            envelope.vault_id.as_str(),
            path.as_ref(),
            envelope.key_name.as_str(),
        ] {
            result.extend_from_slice(&(field.len() as u64).to_le_bytes());
            result.extend_from_slice(field.as_bytes());
        }
        result
    }
    pub fn enroll(&self, password: &str, vault_id: &str, hwnd: isize) -> Result<(), String> {
        self.enroll_with(password, vault_id, hwnd, &WindowsHello)
    }
    fn enroll_with(
        &self,
        password: &str,
        vault_id: &str,
        hwnd: isize,
        protector: &impl Protector,
    ) -> Result<(), String> {
        self.enroll_persisting(password, vault_id, hwnd, protector, |bytes| {
            let parent = self.enrollment_path.parent().ok_or(ERROR)?;
            let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| ERROR)?;
            temp.write_all(bytes).map_err(|_| ERROR)?;
            temp.as_file().sync_all().map_err(|_| ERROR)?;
            temp.persist(&self.enrollment_path).map_err(|_| ERROR)?;
            Ok(())
        })
    }
    fn enroll_persisting(
        &self,
        password: &str,
        vault_id: &str,
        hwnd: isize,
        protector: &impl Protector,
        persist: impl FnOnce(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if password.is_empty() || password.len() > 1024 {
            return Err(ERROR.into());
        }
        let old = self.read()?;
        let name = protector.name()?;
        let mut envelope = Envelope {
            version: VERSION,
            scheme: SCHEME.into(),
            key_name: name.clone(),
            vault_id: vault_id.into(),
            wrapped_key: vec![0; 256],
            nonce: vec![0; 12],
            ciphertext: vec![0; 16],
        };
        envelope.validate()?;
        // The protector cleans keys only after successful creation; a collision must never delete an existing key.
        protector.create(&name, hwnd)?;
        let result = (|| {
            let mut key = Zeroizing::new([0u8; 32]);
            OsRng.try_fill_bytes(key.as_mut()).map_err(|_| ERROR)?;
            envelope.wrapped_key = protector.seal(&name, key.as_ref(), hwnd)?;
            let recovered = protector.open(&name, &envelope.wrapped_key, hwnd)?;
            if recovered.as_slice() != key.as_ref() {
                return Err(ERROR.into());
            }
            OsRng
                .try_fill_bytes(&mut envelope.nonce)
                .map_err(|_| ERROR)?;
            let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| ERROR)?;
            envelope.ciphertext = cipher
                .encrypt(
                    Nonce::from_slice(&envelope.nonce),
                    Payload {
                        msg: password.as_bytes(),
                        aad: &self.aad(&envelope),
                    },
                )
                .map_err(|_| ERROR)?;
            envelope.validate()?;
            let bytes = serde_json::to_vec(&envelope).map_err(|_| ERROR)?;
            if bytes.len() as u64 > MAX_FILE {
                return Err(ERROR.into());
            }
            persist(&bytes)
        })();
        if result.is_err() {
            let _ = protector.delete(&name);
        } else if let Some(old) = old {
            let _ = protector.delete(&old.key_name);
        }
        result
    }
    pub fn unlock(&self, hwnd: isize) -> Result<UnlockedSecret, String> {
        self.unlock_with(hwnd, &WindowsHello)
    }
    fn unlock_with(
        &self,
        hwnd: isize,
        protector: &impl Protector,
    ) -> Result<UnlockedSecret, String> {
        let envelope = self.read()?.ok_or(ERROR)?;
        let key = protector.open(&envelope.key_name, &envelope.wrapped_key, hwnd)?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| ERROR)?;
        let cleartext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&envelope.nonce),
                    Payload {
                        msg: &envelope.ciphertext,
                        aad: &self.aad(&envelope),
                    },
                )
                .map_err(|_| ERROR)?,
        );
        // Avoid String::from_utf8's error type retaining an unzeroized plaintext allocation.
        let password = Zeroizing::new(
            std::str::from_utf8(&cleartext)
                .map_err(|_| ERROR)?
                .to_owned(),
        );
        Ok(UnlockedSecret {
            password,
            vault_id: envelope.vault_id,
        })
    }
    pub fn disable(&self) -> Result<(), String> {
        self.disable_with(&WindowsHello)
    }
    fn disable_with(&self, protector: &impl Protector) -> Result<(), String> {
        // Invalid metadata can always be removed without passing untrusted names to the OS.
        let old = self.read().ok().flatten();
        match std::fs::remove_file(&self.enrollment_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not disable Windows Hello. Try again.".into()),
        }
        if let Some(old) = old {
            protector.delete(&old.key_name).map_err(|_| {
                "Windows Hello was disabled. Windows could not remove its unused key.".to_string()
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct Mock {
        counter: Cell<u128>,
        key: RefCell<Vec<u8>>,
        fail_open: Cell<bool>,
        fail_create: Cell<bool>,
        deleted: RefCell<Vec<String>>,
    }
    impl Protector for Mock {
        fn name(&self) -> Result<String, String> {
            let n = self.counter.get() + 1;
            self.counter.set(n);
            Ok(format!("S-1-5-21//Fakkarni//{}", uuid::Uuid::from_u128(n)))
        }
        fn create(&self, _: &str, _: isize) -> Result<(), String> {
            if self.fail_create.get() {
                return Err("Key already exists".into());
            }
            Ok(())
        }
        fn seal(&self, _: &str, data: &[u8], _: isize) -> Result<Vec<u8>, String> {
            *self.key.borrow_mut() = data.to_vec();
            Ok(vec![5; 256])
        }
        fn open(&self, _: &str, _: &[u8], _: isize) -> Result<Zeroizing<Vec<u8>>, String> {
            if self.fail_open.get() {
                return Err("Cancelled".into());
            }
            Ok(Zeroizing::new(self.key.borrow().clone()))
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.deleted.borrow_mut().push(name.into());
            Ok(())
        }
    }
    const ID: &str = "00000000-0000-0000-0000-000000000001";
    fn setup() -> (tempfile::TempDir, Manager, Mock) {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("hello.json"), dir.path().join("vault.kdbx"));
        (dir, manager, Mock::default())
    }
    #[test]
    fn roundtrip_no_plaintext_and_disable() {
        let (_dir, m, p) = setup();
        assert!(!m.enabled().unwrap());
        m.enroll_with("synthetic test secret", ID, 0, &p).unwrap();
        assert!(m.enabled().unwrap());
        assert!(!std::fs::read_to_string(&m.enrollment_path)
            .unwrap()
            .contains("synthetic test secret"));
        let result = m.unlock_with(0, &p).unwrap();
        assert_eq!(&*result.password, "synthetic test secret");
        assert_eq!(result.vault_id, ID);
        m.disable_with(&p).unwrap();
        assert!(!m.enabled().unwrap());
        assert_eq!(p.deleted.borrow().len(), 1);
    }
    #[test]
    fn tampering_path_binding_and_wrong_key_fail() {
        let (_dir, m, p) = setup();
        m.enroll_with("test", ID, 0, &p).unwrap();
        let other = Manager::new(
            m.enrollment_path.clone(),
            PathBuf::from("different-vault.kdbx"),
        );
        assert!(other.unlock_with(0, &p).is_err());
        let saved_key = p.key.borrow().clone();
        p.key.borrow_mut().fill(0);
        assert!(m.unlock_with(0, &p).is_err());
        *p.key.borrow_mut() = saved_key;
        let mut env = m.read().unwrap().unwrap();
        env.vault_id = "00000000-0000-0000-0000-000000000002".into();
        std::fs::write(&m.enrollment_path, serde_json::to_vec(&env).unwrap()).unwrap();
        assert!(m.unlock_with(0, &p).is_err());
        env.vault_id = ID.into();
        env.ciphertext[0] ^= 1;
        std::fs::write(&m.enrollment_path, serde_json::to_vec(&env).unwrap()).unwrap();
        assert!(m.unlock_with(0, &p).is_err());
        p.key.borrow_mut().fill(0);
        assert!(m.unlock_with(0, &p).is_err());
    }
    #[test]
    fn v2_requires_explicit_scheme_and_authenticates_key_name() {
        let (_dir, m, p) = setup();
        m.enroll_with("synthetic versioned secret", ID, 0, &p)
            .unwrap();
        let original = std::fs::read(&m.enrollment_path).unwrap();
        let mut env = m.read().unwrap().unwrap();
        assert_eq!(env.version, 2);
        assert_eq!(env.scheme, SCHEME);
        env.version = 1;
        std::fs::write(&m.enrollment_path, serde_json::to_vec(&env).unwrap()).unwrap();
        assert!(m.unlock_with(0, &p).is_err());
        env.version = VERSION;
        env.scheme = "RSA-OAEP-SHA256+A256GCM".into();
        std::fs::write(&m.enrollment_path, serde_json::to_vec(&env).unwrap()).unwrap();
        assert!(m.unlock_with(0, &p).is_err());
        env.scheme = SCHEME.into();
        // Mock protector intentionally returns the same AES key for any key name;
        // AES-GCM must independently reject changing this authenticated field.
        env.key_name = "S-1-5-21//Fakkarni//00000000-0000-0000-0000-000000000099".into();
        std::fs::write(&m.enrollment_path, serde_json::to_vec(&env).unwrap()).unwrap();
        assert!(m.unlock_with(0, &p).is_err());
        std::fs::write(&m.enrollment_path, original).unwrap();
        assert_eq!(
            &*m.unlock_with(0, &p).unwrap().password,
            "synthetic versioned secret"
        );
    }
    #[test]
    fn cancellation_keeps_old_enrollment_and_cleans_new_key() {
        let (_dir, m, p) = setup();
        m.enroll_with("old", ID, 0, &p).unwrap();
        let old = std::fs::read(&m.enrollment_path).unwrap();
        p.fail_open.set(true);
        assert!(m.enroll_with("new", ID, 0, &p).is_err());
        assert_eq!(old, std::fs::read(&m.enrollment_path).unwrap());
        assert_eq!(p.deleted.borrow().len(), 1);
        assert!(p.deleted.borrow()[0].ends_with("000000000002"));
    }
    #[test]
    fn create_failure_never_deletes_existing_key() {
        let (_dir, m, p) = setup();
        m.enroll_with("old", ID, 0, &p).unwrap();
        let old = std::fs::read(&m.enrollment_path).unwrap();
        p.fail_create.set(true);
        assert!(m.enroll_with("new", ID, 0, &p).is_err());
        assert!(p.deleted.borrow().is_empty());
        assert_eq!(old, std::fs::read(&m.enrollment_path).unwrap());
        assert_eq!(&*m.unlock_with(0, &p).unwrap().password, "old");
    }
    #[test]
    fn failed_persistence_preserves_old_metadata_and_key() {
        let (_dir, m, p) = setup();
        m.enroll_with("old", ID, 0, &p).unwrap();
        let old = std::fs::read(&m.enrollment_path).unwrap();
        let old_key = p.key.borrow().clone();
        assert!(m
            .enroll_persisting("new", ID, 0, &p, |_| Err(
                "simulated atomic replacement failure".into()
            ))
            .is_err());
        assert_eq!(old, std::fs::read(&m.enrollment_path).unwrap());
        assert_eq!(p.deleted.borrow().len(), 1);
        assert!(p.deleted.borrow()[0].ends_with("000000000002"));
        *p.key.borrow_mut() = old_key;
        assert_eq!(&*m.unlock_with(0, &p).unwrap().password, "old");
    }
    #[test]
    fn limits_and_strict_schema() {
        let (_dir, m, p) = setup();
        assert!(m.enroll_with(&"x".repeat(1025), ID, 0, &p).is_err());
        assert!(m.enroll_with("test", "bad", 0, &p).is_err());
        m.enroll_with(&"x".repeat(1024), ID, 0, &p).unwrap();
        assert_eq!(m.unlock_with(0, &p).unwrap().password.len(), 1024);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&m.enrollment_path).unwrap()).unwrap();
        value["extra"] = true.into();
        std::fs::write(&m.enrollment_path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(m.enabled().is_err());
        std::fs::write(&m.enrollment_path, vec![b' '; MAX_FILE as usize + 1]).unwrap();
        assert!(m.enabled().is_err());
        m.disable_with(&p).unwrap();
        assert!(!m.enabled().unwrap());
    }
}
