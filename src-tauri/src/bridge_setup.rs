//! Explicit native browser-bridge repair, performed in the desktop user's
//! Windows session. No shell, repository scripts, or credential files are used.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::Mutex,
};
use tauri::Manager;

const HOST_NAME: &str = "org.fakkarni.bridge";
const EXTENSION_ID: &str = "kieggakkiiegglikoiojkbfabpmmahep";
const MAX_HOST_BYTES: u64 = 256 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024;
const REGISTRY_PATHS: [&str; 3] = [
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\org.fakkarni.bridge",
    r"Software\Google\Chrome\NativeMessagingHosts\org.fakkarni.bridge",
    r"Software\Microsoft\Edge\NativeMessagingHosts\org.fakkarni.bridge",
];
static OPERATION_GATE: Mutex<()> = Mutex::new(());

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RepairReport {
    pub browsers: Vec<String>,
    pub restart_required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RegistryView {
    Bits32,
    Bits64,
}
const VIEWS: [RegistryView; 2] = [RegistryView::Bits32, RegistryView::Bits64];
trait Registrar {
    fn matches(&self, path: &str, view: RegistryView, manifest: &str) -> Result<bool, String>;
    fn write(&self, path: &str, view: RegistryView, manifest: &str) -> Result<(), String>;
}
struct CurrentUserRegistrar;

#[cfg(windows)]
impl Registrar for CurrentUserRegistrar {
    fn matches(&self, path: &str, view: RegistryView, manifest: &str) -> Result<bool, String> {
        use windows_registry::{Type, CURRENT_USER};
        let key = match CURRENT_USER
            .options()
            .read()
            .access(view.access())
            .open(path)
        {
            Ok(key) => key,
            Err(error) if matches!(error.code().0 as u32, 0x80070002 | 0x80070003) => {
                return Ok(false)
            }
            Err(_) => return Err("Could not read this user's browser bridge registration.".into()),
        };
        match key.get_type("") {
            Ok(Type::String) => key
                .get_string("")
                .map(|value| value == manifest)
                .map_err(|_| "Could not read browser bridge registration.".into()),
            Ok(_) => Ok(false),
            Err(error) if matches!(error.code().0 as u32, 0x80070002 | 0x80070003) => Ok(false),
            Err(_) => Err("Could not verify browser bridge registration.".into()),
        }
    }
    fn write(&self, path: &str, view: RegistryView, manifest: &str) -> Result<(), String> {
        use windows_registry::CURRENT_USER;
        // Create each missing ancestor explicitly. A clean PC may have neither
        // the browser vendor key nor NativeMessagingHosts yet.
        let mut prefix = String::new();
        let mut final_key = None;
        for component in path.split('\\') {
            if !prefix.is_empty() {
                prefix.push('\\');
            }
            prefix.push_str(component);
            final_key = Some(CURRENT_USER.options().read().write().create().access(view.access()).open(&prefix)
                .map_err(|_| "Could not create this user's browser bridge registration. No administrator access should be needed.".to_owned())?);
        }
        final_key
            .ok_or("Invalid browser bridge registration path.")?
            .set_string("", manifest)
            .map_err(|_| {
                "Could not write browser bridge registration for this Windows user.".into()
            })
    }
}
#[cfg(windows)]
impl RegistryView {
    fn access(self) -> u32 {
        match self {
            Self::Bits32 => 0x0200,
            Self::Bits64 => 0x0100,
        }
    }
}
#[cfg(not(windows))]
impl Registrar for CurrentUserRegistrar {
    fn matches(&self, _: &str, _: RegistryView, _: &str) -> Result<bool, String> {
        Err("Browser bridge repair is available on Windows.".into())
    }
    fn write(&self, _: &str, _: RegistryView, _: &str) -> Result<(), String> {
        Err("Browser bridge repair is available on Windows.".into())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostManifest {
    name: String,
    description: String,
    path: String,
    #[serde(rename = "type")]
    transport: String,
    allowed_origins: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaunchConfig {
    desktop_exe_full_path: String,
}

fn files_equal(first: &Path, second: &Path) -> std::io::Result<bool> {
    let mut first = fs::File::open(first)?;
    let mut second = fs::File::open(second)?;
    if first.metadata()?.len() != second.metadata()?.len() {
        return Ok(false);
    }
    let mut left = [0u8; 65536];
    let mut right = [0u8; 65536];
    loop {
        let length = first.read(&mut left)?;
        if length == 0 {
            return Ok(true);
        }
        second.read_exact(&mut right[..length])?;
        if left[..length] != right[..length] {
            return Ok(false);
        }
    }
}
fn install_host(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::metadata(source).map_err(|_| "The browser bridge executable is missing beside Fakkarni. Keep fakkarni-native-host.exe in the same folder as the desktop application.".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_HOST_BYTES {
        return Err("The browser bridge executable is invalid or too large.".into());
    }
    if files_equal(source, target).unwrap_or(false) {
        return Ok(());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(
        target.parent().ok_or("Invalid browser bridge directory.")?,
    )
    .map_err(|_| "Could not prepare the browser bridge executable.".to_owned())?;
    let source_file = fs::File::open(source)
        .map_err(|_| "Could not read the browser bridge executable.".to_owned())?;
    let length = std::io::copy(&mut source_file.take(MAX_HOST_BYTES + 1), &mut temporary)
        .map_err(|_| "Could not copy the browser bridge executable.".to_owned())?;
    if length == 0
        || length > MAX_HOST_BYTES
        || !files_equal(source, temporary.path()).unwrap_or(false)
    {
        return Err("The browser bridge executable changed during repair. Try again.".into());
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "Could not flush the browser bridge executable.".to_owned())?;
    temporary.persist(target).map_err(|_| "Could not replace the browser bridge executable. Close Brave and Edge completely, then run Repair browser connection again. The previous installed executable was preserved.".to_owned())?;
    if !files_equal(source, target).unwrap_or(false) {
        return Err("The installed browser bridge executable could not be verified.".into());
    }
    Ok(())
}
fn install_metadata(target: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err("Browser bridge metadata exceeds its size limit.".into());
    }
    if fs::metadata(target).is_ok_and(|metadata| metadata.len() == bytes.len() as u64) {
        let mut existing = Vec::new();
        if fs::File::open(target)
            .and_then(|file| {
                file.take(MAX_METADATA_BYTES as u64 + 1)
                    .read_to_end(&mut existing)
            })
            .is_ok()
            && existing == bytes
        {
            return Ok(());
        }
    }
    let mut temporary = tempfile::NamedTempFile::new_in(
        target.parent().ok_or("Invalid browser bridge directory.")?,
    )
    .map_err(|_| "Could not prepare browser bridge metadata.".to_owned())?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "Could not write browser bridge metadata.".to_owned())?;
    temporary.persist(target).map_err(|_| {
        "Could not atomically install browser bridge metadata. The previous file was preserved."
            .to_owned()
    })?;
    let installed = fs::read(target)
        .map_err(|_| "Could not verify installed browser bridge metadata.".to_owned())?;
    if installed != bytes {
        return Err("Installed browser bridge metadata could not be verified.".into());
    }
    Ok(())
}

fn install(
    desktop: &Path,
    directory: &Path,
    registry: &impl Registrar,
) -> Result<RepairReport, String> {
    if !desktop.is_absolute()
        || !desktop.is_file()
        || desktop
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
    {
        return Err("Could not locate the current desktop application executable.".into());
    }
    let source = desktop
        .parent()
        .ok_or("Could not locate the browser bridge executable.")?
        .join("fakkarni-native-host.exe");
    // Check the sidecar before creating files or registry values.
    if !source.is_file() {
        return Err("The browser bridge executable is missing beside Fakkarni. Keep fakkarni-native-host.exe in the same folder as the desktop application.".into());
    }
    fs::create_dir_all(directory)
        .map_err(|_| "Could not create this user's browser bridge directory.".to_owned())?;
    let target = directory.join("fakkarni-native-host.exe");
    install_host(&source, &target)?;
    let desktop_text = desktop
        .to_str()
        .ok_or("The desktop executable path is not valid Unicode.")?;
    let target_text = target
        .to_str()
        .ok_or("The browser bridge path is not valid Unicode.")?;
    let manifest_path = directory.join("org.fakkarni.bridge.json");
    let manifest_text = manifest_path
        .to_str()
        .ok_or("The browser manifest path is not valid Unicode.")?;
    let config = LaunchConfig {
        desktop_exe_full_path: desktop_text.into(),
    };
    let manifest = HostManifest {
        name: HOST_NAME.into(),
        description: "Fakkarni local account suggestions and autofill".into(),
        path: target_text.into(),
        transport: "stdio".into(),
        allowed_origins: vec![format!("chrome-extension://{EXTENSION_ID}/")],
    };
    install_metadata(
        &directory.join("desktop-path.json"),
        &serde_json::to_vec_pretty(&config)
            .map_err(|_| "Could not prepare browser launch configuration.")?,
    )?;
    install_metadata(
        &manifest_path,
        &serde_json::to_vec_pretty(&manifest).map_err(|_| "Could not prepare browser manifest.")?,
    )?;
    for path in REGISTRY_PATHS {
        for view in VIEWS {
            if !registry.matches(path, view, manifest_text)? {
                registry.write(path, view, manifest_text)?;
            }
            if !registry.matches(path, view, manifest_text)? {
                return Err(
                    "Browser bridge registry verification failed. Repair did not complete.".into(),
                );
            }
        }
    }
    Ok(RepairReport {
        browsers: vec!["Brave".into(), "Edge".into()],
        restart_required: true,
    })
}

#[tauri::command]
pub(super) async fn repair_browser_bridge(app: tauri::AppHandle) -> Result<RepairReport, String> {
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|_| "Could not locate this user's application data directory.".to_owned())?
        .join("native-messaging");
    let desktop = std::env::current_exe()
        .map_err(|_| "Could not locate the desktop executable.".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = OPERATION_GATE
            .try_lock()
            .map_err(|_| "Another browser bridge repair is in progress.".to_owned())?;
        let _shared_gate = shared_gate::acquire()?;
        install(&desktop, &directory, &CurrentUserRegistrar)
    })
    .await
    .map_err(|_| "Browser bridge repair stopped unexpectedly. Try again.".to_owned())?
}

#[cfg(windows)]
mod shared_gate {
    use std::{ffi::c_void, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::{
            CreateMutexW, GetCurrentProcess, OpenProcessToken, ReleaseMutex, WaitForSingleObject,
        },
    };
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct Local(*mut c_void);
    impl Drop for Local {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    pub(super) struct Guard(HANDLE);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
    fn current_sid() -> Result<String, String> {
        unsafe {
            let mut token = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err("Could not identify the Windows user for browser repair.".into());
            }
            let token = Token(token);
            let mut needed = 0;
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed);
            if needed < std::mem::size_of::<TOKEN_USER>() as u32 || needed > 65536 {
                return Err("Could not identify the Windows user for browser repair.".into());
            }
            let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
            if GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ) == 0
            {
                return Err("Could not identify the Windows user for browser repair.".into());
            }
            let token_user = &*storage.as_ptr().cast::<TOKEN_USER>();
            let mut text = ptr::null_mut();
            if ConvertSidToStringSidW(token_user.User.Sid, &mut text) == 0 {
                return Err("Could not identify the Windows user for browser repair.".into());
            }
            let _local = Local(text.cast());
            let mut length = 0;
            while length < 256 && *text.add(length) != 0 {
                length += 1;
            }
            if length == 256 {
                return Err("Could not identify the Windows user for browser repair.".into());
            }
            String::from_utf16(std::slice::from_raw_parts(text, length))
                .map_err(|_| "Could not identify the Windows user for browser repair.".into())
        }
    }
    pub(super) fn acquire() -> Result<Guard, String> {
        let name: Vec<u16> = format!("Local\\FakkarniBrowserRegistration-{}\0", current_sid()?)
            .encode_utf16()
            .collect();
        unsafe {
            let handle = CreateMutexW(ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err("Could not coordinate browser bridge repair.".into());
            }
            match WaitForSingleObject(handle, 10000) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Guard(handle)),
                _ => {
                    CloseHandle(handle);
                    Err("Another browser bridge installation is in progress. Retry shortly.".into())
                }
            }
        }
    }
}
#[cfg(not(windows))]
mod shared_gate {
    pub(super) fn acquire() -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
        path::PathBuf,
    };
    #[derive(Default)]
    struct FakeRegistrar {
        values: RefCell<BTreeMap<(RegistryView, String), String>>,
        writes: Cell<usize>,
        fail: Cell<bool>,
        discard_write: Cell<bool>,
    }
    impl Registrar for FakeRegistrar {
        fn matches(&self, path: &str, view: RegistryView, manifest: &str) -> Result<bool, String> {
            Ok(self
                .values
                .borrow()
                .get(&(view, path.into()))
                .is_some_and(|value| value == manifest))
        }
        fn write(&self, path: &str, view: RegistryView, manifest: &str) -> Result<(), String> {
            if self.fail.get() {
                return Err("Synthetic registry access denied".into());
            }
            self.writes.set(self.writes.get() + 1);
            if !self.discard_write.get() {
                self.values
                    .borrow_mut()
                    .insert((view, path.into()), manifest.into());
            }
            Ok(())
        }
    }
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let application = directory.path().join("Synthetic application folder");
        fs::create_dir(&application).unwrap();
        let desktop = application.join("fakkarni.exe");
        fs::write(&desktop, b"Synthetic desktop executable").unwrap();
        fs::write(
            application.join("fakkarni-native-host.exe"),
            b"Synthetic native host executable",
        )
        .unwrap();
        let installed = directory
            .path()
            .join("Synthetic user data/native-messaging");
        (directory, desktop, installed)
    }
    #[test]
    fn manifest_identity_matches_extension() {
        let identity: serde_json::Value =
            serde_json::from_str(include_str!("../../extension/identity.json")).unwrap();
        assert_eq!(identity["nativeHost"], HOST_NAME);
        assert_eq!(identity["extensionId"], EXTENSION_ID);
    }
    #[cfg(windows)]
    #[test]
    fn native_registrar_creates_missing_parents_and_repairs_value_type() {
        use windows_registry::CURRENT_USER;
        struct SyntheticBranch(uuid::Uuid);
        impl SyntheticBranch {
            fn relative(&self) -> String {
                format!("FakkarniBridgeRepairTests\\{}", self.0.simple())
            }
        }
        impl Drop for SyntheticBranch {
            fn drop(&mut self) {
                for view in VIEWS {
                    if let Ok(software) = CURRENT_USER
                        .options()
                        .read()
                        .write()
                        .access(view.access())
                        .open("Software")
                    {
                        let _ = software.remove_tree(self.relative());
                    }
                }
            }
        }
        // Only a fresh, UUID-scoped synthetic branch is created or removed.
        // Production browser registrations and application files are untouched.
        let branch = SyntheticBranch(uuid::Uuid::new_v4());
        let path = format!(
            "Software\\{}\\missing\\parents\\NativeMessagingHosts\\synthetic",
            branch.relative()
        );
        let manifest = r"C:\Synthetic fixture\bridge.json";
        let registry = CurrentUserRegistrar;
        for view in VIEWS {
            assert!(!registry.matches(&path, view, manifest).unwrap());
        }
        for view in VIEWS {
            registry.write(&path, view, manifest).unwrap();
            assert!(registry.matches(&path, view, manifest).unwrap());
            let key = CURRENT_USER
                .options()
                .read()
                .write()
                .access(view.access())
                .open(&path)
                .unwrap();
            key.set_u32("", 7).unwrap();
            assert!(!registry.matches(&path, view, manifest).unwrap());
            registry.write(&path, view, manifest).unwrap();
            assert!(registry.matches(&path, view, manifest).unwrap());
        }
    }
    #[test]
    fn fresh_install_verifies_files_and_six_registry_locations_without_real_registry() {
        let (_directory, desktop, installed) = fixture();
        let registry = FakeRegistrar::default();
        let report = install(&desktop, &installed, &registry).unwrap();
        assert_eq!(report.browsers, ["Brave", "Edge"]);
        assert!(report.restart_required);
        assert_eq!(registry.writes.get(), 6);
        assert_eq!(registry.values.borrow().len(), 6);
        let manifest: HostManifest =
            serde_json::from_slice(&fs::read(installed.join("org.fakkarni.bridge.json")).unwrap())
                .unwrap();
        assert_eq!(manifest.name, HOST_NAME);
        assert_eq!(manifest.transport, "stdio");
        assert_eq!(
            manifest.allowed_origins,
            [format!("chrome-extension://{EXTENSION_ID}/")]
        );
        assert_eq!(
            manifest.path,
            installed.join("fakkarni-native-host.exe").to_str().unwrap()
        );
        assert!(files_equal(
            &desktop.parent().unwrap().join("fakkarni-native-host.exe"),
            Path::new(&manifest.path)
        )
        .unwrap());
        let config: LaunchConfig =
            serde_json::from_slice(&fs::read(installed.join("desktop-path.json")).unwrap())
                .unwrap();
        assert_eq!(config.desktop_exe_full_path, desktop.to_str().unwrap());
    }
    #[test]
    fn repeated_install_preserves_files_and_registry_values() {
        let (_directory, desktop, installed) = fixture();
        let registry = FakeRegistrar::default();
        install(&desktop, &installed, &registry).unwrap();
        let paths: Vec<_> = fs::read_dir(&installed)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let before: Vec<_> = paths
            .iter()
            .map(|path| {
                (
                    fs::metadata(path).unwrap().modified().unwrap(),
                    fs::read(path).unwrap(),
                )
            })
            .collect();
        install(&desktop, &installed, &registry).unwrap();
        assert_eq!(registry.writes.get(), 6);
        for (path, (modified, bytes)) in paths.iter().zip(before) {
            assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn missing_host_and_failed_registration_cannot_report_success() {
        let (_directory, desktop, installed) = fixture();
        let source = desktop.parent().unwrap().join("fakkarni-native-host.exe");
        fs::remove_file(&source).unwrap();
        let registry = FakeRegistrar::default();
        assert!(install(&desktop, &installed, &registry)
            .unwrap_err()
            .contains("missing beside"));
        assert!(!installed.exists());
        assert_eq!(registry.writes.get(), 0);
        fs::write(&source, b"Synthetic restored sidecar").unwrap();
        registry.discard_write.set(true);
        assert!(install(&desktop, &installed, &registry).is_err());
        assert!(registry.values.borrow().is_empty());
        registry.fail.set(true);
        assert!(install(&desktop, &installed, &registry).is_err());
    }
    #[test]
    fn metadata_limits_and_failed_replacement_preserve_previous_file() {
        let (_directory, _desktop, installed) = fixture();
        fs::create_dir_all(&installed).unwrap();
        let path = installed.join("org.fakkarni.bridge.json");
        fs::write(&path, b"Synthetic previous metadata").unwrap();
        assert!(install_metadata(&path, &vec![0; MAX_METADATA_BYTES + 1]).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"Synthetic previous metadata");
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let _blocker = fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&path)
                .unwrap();
            assert!(install_metadata(&path, b"Synthetic updated metadata").is_err());
            assert_eq!(fs::read(&path).unwrap(), b"Synthetic previous metadata");
        }
    }
    #[cfg(windows)]
    #[test]
    fn changed_in_use_host_fails_atomically_and_unchanged_in_use_host_is_safe() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_directory, desktop, installed) = fixture();
        let registry = FakeRegistrar::default();
        install(&desktop, &installed, &registry).unwrap();
        let target = installed.join("fakkarni-native-host.exe");
        let original = fs::read(&target).unwrap();
        let _blocker = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();
        install(&desktop, &installed, &registry).unwrap();
        fs::write(
            desktop.parent().unwrap().join("fakkarni-native-host.exe"),
            b"Synthetic changed host",
        )
        .unwrap();
        assert!(install(&desktop, &installed, &registry).is_err());
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(registry.writes.get(), 6);
    }
}
