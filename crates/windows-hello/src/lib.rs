//! Device-bound Windows Hello wrapping of 32-byte keys.
//!
//! Passport KSP's mandatory-gesture and implementation-type properties are
//! provider-specific. Unsupported properties fail closed. RSA-OAEP SHA-256 is
//! required; no padding or software-provider fallback is attempted.

use zeroize::Zeroizing;

const WRAPPING_KEY_BYTES: usize = 32;
const RSA_BYTES: usize = 256;

#[cfg(any(windows, test))]
fn validate_name_for_sid(name: &str, sid: &str) -> Result<(), String> {
    let prefix = format!("{sid}//Fakkarni//");
    let suffix = name
        .strip_prefix(&prefix)
        .ok_or("Invalid Windows Hello key name.")?;
    let id = uuid::Uuid::parse_str(suffix).map_err(|_| "Invalid Windows Hello key name.")?;
    if id.get_version_num() != 4 || id.to_string() != suffix {
        return Err("Invalid Windows Hello key name.".into());
    }
    Ok(())
}

fn validate_plaintext(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != WRAPPING_KEY_BYTES {
        return Err("Windows Hello accepts only a 32-byte wrapping key.".into());
    }
    Ok(())
}

fn validate_ciphertext(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != RSA_BYTES {
        return Err("Invalid Windows Hello wrapped-key length.".into());
    }
    Ok(())
}

/// Checks configured Hello availability without displaying any prompt.
pub fn availability() -> Result<bool, String> {
    platform::availability()
}
pub fn new_key_name() -> Result<String, String> {
    Ok(format!(
        "{}//Fakkarni//{}",
        platform::current_sid()?,
        uuid::Uuid::new_v4()
    ))
}
pub fn create(key_name: &str, hwnd: isize) -> Result<(), String> {
    platform::create(key_name, hwnd)
}
pub fn seal(key_name: &str, plaintext: &[u8], hwnd: isize) -> Result<Vec<u8>, String> {
    validate_plaintext(plaintext)?;
    platform::seal(key_name, plaintext, hwnd)
}
pub fn open(key_name: &str, ciphertext: &[u8], hwnd: isize) -> Result<Zeroizing<Vec<u8>>, String> {
    validate_ciphertext(ciphertext)?;
    platform::open(key_name, ciphertext, hwnd)
}
pub fn delete(key_name: &str) -> Result<(), String> {
    platform::delete(key_name)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows::Security::Credentials::UI::{
        UserConsentVerifier, UserConsentVerifierAvailability,
    };

    type Handle = usize;
    const SILENT: u32 = 0x40;
    const OAEP: u32 = 4;
    const CACHE_PROPERTY: &str = "NgcCacheType";
    const LEGACY_CACHE_PROPERTY: &str = "NgcCacheTypeProperty";
    #[repr(C)]
    struct OaepInfo {
        algorithm: *const u16,
        label: *const u8,
        label_len: u32,
    }
    #[link(name = "ncrypt")]
    unsafe extern "system" {
        fn NCryptOpenStorageProvider(out: *mut Handle, name: *const u16, flags: u32) -> i32;
        fn NCryptCreatePersistedKey(
            provider: Handle,
            out: *mut Handle,
            alg: *const u16,
            name: *const u16,
            spec: u32,
            flags: u32,
        ) -> i32;
        fn NCryptFinalizeKey(key: Handle, flags: u32) -> i32;
        fn NCryptOpenKey(
            provider: Handle,
            out: *mut Handle,
            name: *const u16,
            spec: u32,
            flags: u32,
        ) -> i32;
        fn NCryptSetProperty(
            key: Handle,
            name: *const u16,
            input: *const u8,
            len: u32,
            flags: u32,
        ) -> i32;
        fn NCryptGetProperty(
            key: Handle,
            name: *const u16,
            output: *mut u8,
            len: u32,
            written: *mut u32,
            flags: u32,
        ) -> i32;
        fn NCryptEncrypt(
            key: Handle,
            input: *const u8,
            len: u32,
            padding: *const OaepInfo,
            output: *mut u8,
            cap: u32,
            written: *mut u32,
            flags: u32,
        ) -> i32;
        fn NCryptDecrypt(
            key: Handle,
            input: *const u8,
            len: u32,
            padding: *const OaepInfo,
            output: *mut u8,
            cap: u32,
            written: *mut u32,
            flags: u32,
        ) -> i32;
        fn NCryptDeleteKey(key: Handle, flags: u32) -> i32;
        fn NCryptFreeObject(handle: Handle) -> i32;
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn GetTokenInformation(
            token: *mut c_void,
            class: u32,
            output: *mut c_void,
            len: u32,
            needed: *mut u32,
        ) -> i32;
        fn ConvertSidToStringSidW(sid: *const c_void, output: *mut *mut u16) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
    #[link(name = "runtimeobject")]
    unsafe extern "system" {
        fn RoInitialize(kind: u32) -> i32;
        fn RoUninitialize();
    }
    struct WinRtApartment(bool);
    impl WinRtApartment {
        fn initialize() -> Result<Self, String> {
            // S_OK and S_FALSE both increment the apartment initialization
            // count. RPC_E_CHANGED_MODE means a usable STA already exists;
            // this call did not increment its count.
            match unsafe { RoInitialize(1) } as u32 {
                0 | 1 => Ok(Self(true)),
                0x80010106 => Ok(Self(false)),
                _ => Err("Cannot initialize Windows Hello availability checking.".into()),
            }
        }
    }
    impl Drop for WinRtApartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { RoUninitialize() };
            }
        }
    }
    struct Owned(Handle);
    impl Drop for Owned {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    NCryptFreeObject(self.0);
                }
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn status(code: i32, operation: &str) -> Result<(), String> {
        if code == 0 {
            Ok(())
        } else if code as u32 == 0x80090036 || code as u32 == 0x800704c7 {
            Err("Windows Hello was cancelled.".into())
        } else {
            Err(format!(
                "Windows Hello {operation} failed (0x{:08X}). Use your master password.",
                code as u32
            ))
        }
    }
    pub fn current_sid() -> Result<String, String> {
        // TOKEN_USER starts with SID_AND_ATTRIBUTES; use an aligned buffer.
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), 8, &mut token) == 0 {
                return Err("Cannot identify the current Windows user.".into());
            }
            let mut needed = 0;
            GetTokenInformation(token, 1, null_mut(), 0, &mut needed);
            if needed == 0 || needed > 65536 {
                CloseHandle(token);
                return Err("Cannot identify the current Windows user.".into());
            }
            let mut storage =
                vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
            let ok =
                GetTokenInformation(token, 1, storage.as_mut_ptr().cast(), needed, &mut needed);
            CloseHandle(token);
            if ok == 0 {
                return Err("Cannot identify the current Windows user.".into());
            }
            let sid = *(storage.as_ptr() as *const *const c_void);
            let mut text = null_mut();
            if ConvertSidToStringSidW(sid, &mut text) == 0 {
                return Err("Cannot identify the current Windows user.".into());
            }
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let result = String::from_utf16(std::slice::from_raw_parts(text, len))
                .map_err(|_| "Cannot identify the current Windows user.".into());
            LocalFree(text.cast());
            result
        }
    }
    pub fn availability() -> Result<bool, String> {
        let _apartment = WinRtApartment::initialize()?;
        let request = UserConsentVerifier::CheckAvailabilityAsync()
            .map_err(|_| "Cannot check Windows Hello availability.".to_string())?;
        let result = request
            .join()
            .map_err(|_| "Cannot check Windows Hello availability.".to_string())?;
        Ok(result == UserConsentVerifierAvailability::Available)
    }
    fn check_name(name: &str) -> Result<(), String> {
        validate_name_for_sid(name, &current_sid()?)
    }
    fn provider() -> Result<Owned, String> {
        let mut h = 0;
        let result = unsafe {
            NCryptOpenStorageProvider(
                &mut h,
                wide("Microsoft Passport Key Storage Provider").as_ptr(),
                0,
            )
        };
        let owned = Owned(h);
        status(result, "provider access")?;
        Ok(owned)
    }
    fn set(key: &Owned, name: &str, bytes: &[u8]) -> Result<(), String> {
        status(
            unsafe {
                NCryptSetProperty(
                    key.0,
                    wide(name).as_ptr(),
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    0,
                )
            },
            "key policy setup",
        )
    }
    fn set_number(key: &Owned, name: &str, number: u32) -> Result<(), String> {
        set(key, name, &number.to_le_bytes())
    }
    fn number(key: &Owned, name: &str) -> Result<u32, String> {
        let mut bytes = [0; 4];
        let mut written = 0;
        status(
            unsafe {
                NCryptGetProperty(
                    key.0,
                    wide(name).as_ptr(),
                    bytes.as_mut_ptr(),
                    4,
                    &mut written,
                    0,
                )
            },
            "key policy verification",
        )?;
        if written != 4 {
            return Err("Windows Hello key policy could not be verified.".into());
        }
        Ok(u32::from_le_bytes(bytes))
    }
    fn verify(key: &Owned) -> Result<(), String> {
        let usage = number(key, "Key Usage")?;
        let cache = number(key, CACHE_PROPERTY).or_else(|_| number(key, LEGACY_CACHE_PROPERTY))?;
        if usage & 1 == 0
            || usage & 8 != 0
            || cache != 1
            || number(key, "Export Policy")? != 0
            || number(key, "Length")? != 2048
            || number(key, "NgcKeyImplType")? & 1 == 0
        {
            return Err("Windows Hello requires a non-exportable TPM key and mandatory verification. Use your master password.".into());
        }
        Ok(())
    }
    fn ui(key: &Owned, hwnd: isize) -> Result<(), String> {
        if hwnd == 0 {
            return Err("Windows Hello requires an application window.".into());
        }
        set(key, "HWND Handle", &hwnd.to_le_bytes())?;
        let context = wide("Unlock your Fakkarni vault");
        let bytes =
            unsafe { std::slice::from_raw_parts(context.as_ptr().cast(), context.len() * 2) };
        set(key, "Use Context", bytes)
    }
    fn existing(name: &str) -> Result<(Owned, Owned), String> {
        check_name(name)?;
        let provider = provider()?;
        let mut h = 0;
        let result = unsafe { NCryptOpenKey(provider.0, &mut h, wide(name).as_ptr(), 0, SILENT) };
        let key = Owned(h);
        status(result, "key access")?;
        verify(&key)?;
        Ok((provider, key))
    }
    pub fn create(name: &str, hwnd: isize) -> Result<(), String> {
        check_name(name)?;
        if !availability()? {
            return Err("Set up Windows Hello in Windows Settings first.".into());
        }
        let provider = provider()?;
        let mut h = 0;
        let result = unsafe {
            NCryptCreatePersistedKey(
                provider.0,
                &mut h,
                wide("RSA").as_ptr(),
                wide(name).as_ptr(),
                0,
                0,
            )
        };
        let mut key = Owned(h);
        status(result, "key creation")?;
        let configured = (|| {
            set_number(&key, "Length", 2048)?;
            set_number(&key, "Key Usage", 3)?;
            set_number(&key, "Export Policy", 0)?;
            set_number(&key, CACHE_PROPERTY, 1)
                .or_else(|_| set_number(&key, LEGACY_CACHE_PROPERTY, 1))?;
            ui(&key, hwnd)?;
            status(unsafe { NCryptFinalizeKey(key.0, 0) }, "key creation")?;
            verify(&key)
        })();
        if configured.is_err() && unsafe { NCryptDeleteKey(key.0, SILENT) } == 0 {
            key.0 = 0;
        }
        configured
    }
    pub fn seal(name: &str, plaintext: &[u8], hwnd: isize) -> Result<Vec<u8>, String> {
        let (_provider, key) = existing(name)?;
        ui(&key, hwnd)?;
        let sha = wide("SHA256");
        let info = OaepInfo {
            algorithm: sha.as_ptr(),
            label: null(),
            label_len: 0,
        };
        let mut result = vec![0; RSA_BYTES];
        let mut written = 0;
        status(
            unsafe {
                NCryptEncrypt(
                    key.0,
                    plaintext.as_ptr(),
                    plaintext.len() as u32,
                    &info,
                    result.as_mut_ptr(),
                    RSA_BYTES as u32,
                    &mut written,
                    OAEP | SILENT,
                )
            },
            "key wrapping",
        )?;
        if written as usize != RSA_BYTES {
            return Err("Windows Hello returned an invalid wrapped key.".into());
        }
        Ok(result)
    }
    pub fn open(name: &str, ciphertext: &[u8], hwnd: isize) -> Result<Zeroizing<Vec<u8>>, String> {
        let (_provider, key) = existing(name)?;
        ui(&key, hwnd)?;
        set_number(&key, "PinCacheIsGestureRequired", 1)?;
        let sha = wide("SHA256");
        let info = OaepInfo {
            algorithm: sha.as_ptr(),
            label: null(),
            label_len: 0,
        };
        let mut result = Zeroizing::new(vec![0; RSA_BYTES]);
        let mut written = 0;
        status(
            unsafe {
                NCryptDecrypt(
                    key.0,
                    ciphertext.as_ptr(),
                    ciphertext.len() as u32,
                    &info,
                    result.as_mut_ptr(),
                    RSA_BYTES as u32,
                    &mut written,
                    OAEP,
                )
            },
            "unlock",
        )?;
        if written as usize != WRAPPING_KEY_BYTES {
            return Err("Windows Hello returned an invalid wrapping key.".into());
        }
        // Zero the unused allocation before truncation, including on success.
        use zeroize::Zeroize;
        result[WRAPPING_KEY_BYTES..].zeroize();
        result.truncate(WRAPPING_KEY_BYTES);
        Ok(result)
    }
    pub fn delete(name: &str) -> Result<(), String> {
        // Deletion may clean up an enrollment with now-unsupported properties.
        check_name(name)?;
        let provider = provider()?;
        let mut h = 0;
        let code = unsafe { NCryptOpenKey(provider.0, &mut h, wide(name).as_ptr(), 0, SILENT) };
        let mut key = Owned(h);
        if matches!(code as u32, 0x8009000d | 0x80090011 | 0x80090016) {
            return Ok(());
        }
        status(code, "key access")?;
        status(unsafe { NCryptDeleteKey(key.0, SILENT) }, "key deletion")?;
        key.0 = 0;
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    fn unsupported<T>() -> Result<T, String> {
        Err("Windows Hello is available only on Windows.".into())
    }
    pub fn availability() -> Result<bool, String> {
        Ok(false)
    }
    pub fn current_sid() -> Result<String, String> {
        unsupported()
    }
    pub fn create(_: &str, _: isize) -> Result<(), String> {
        unsupported()
    }
    pub fn seal(_: &str, _: &[u8], _: isize) -> Result<Vec<u8>, String> {
        unsupported()
    }
    pub fn open(_: &str, _: &[u8], _: isize) -> Result<Zeroizing<Vec<u8>>, String> {
        unsupported()
    }
    pub fn delete(_: &str) -> Result<(), String> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SID: &str = "S-1-5-21-111-222-333-1001";
    #[test]
    fn accepts_only_canonical_v4_enrollment_names_for_current_user() {
        let id = uuid::Uuid::new_v4();
        let valid = format!("{SID}//Fakkarni//{id}");
        assert!(validate_name_for_sid(&valid, SID).is_ok());
        for invalid in [
            valid.replace(SID, "S-1-5-21-999"),
            valid.replace("Fakkarni", "Windows"),
            format!("{valid}/other"),
            format!("{SID}//Fakkarni//00000000-0000-0000-0000-000000000000"),
        ] {
            assert!(validate_name_for_sid(&invalid, SID).is_err());
        }
    }
    #[test]
    fn bounds_are_fixed_before_any_os_operation() {
        assert!(validate_plaintext(&[0; 32]).is_ok());
        assert!(validate_plaintext(&[0; 31]).is_err());
        assert!(validate_plaintext(&[0; 33]).is_err());
        assert!(validate_ciphertext(&[0; 256]).is_ok());
        assert!(validate_ciphertext(&[0; 255]).is_err());
        assert!(validate_ciphertext(&[0; 257]).is_err());
    }
}
