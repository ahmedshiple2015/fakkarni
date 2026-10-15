//! Local lifecycle preferences and an explicit opt-in Windows startup entry.
use crate::Shared;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};

const MAX_SETTINGS_BYTES: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppSettings {
    pub start_with_windows: bool,
    pub close_to_tray: bool,
    pub start_minimized: bool,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            close_to_tray: true,
            start_minimized: false,
        }
    }
}

pub struct LifecycleState {
    settings: Mutex<AppSettings>,
    path: PathBuf,
    quitting: AtomicBool,
}

fn read_settings(path: &Path) -> Result<AppSettings, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppSettings::default())
        }
        Err(_) => return Err("Could not read application settings.".into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read application settings.")?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err("Application settings are too large.".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Application settings are invalid.".into())
}

fn persist_settings(path: &Path, settings: &AppSettings) -> Result<(), String> {
    let directory = path.parent().ok_or("Invalid settings location.")?;
    std::fs::create_dir_all(directory).map_err(|_| "Could not create the settings directory.")?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|_| "Could not serialize application settings.")?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|_| "Could not prepare application settings.")?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "Could not write application settings.")?;
    temporary
        .persist(path)
        .map_err(|_| "Could not save application settings.")?;
    Ok(())
}

trait StartupRegistration {
    fn read(&self) -> Result<Option<String>, String>;
    fn write(&self, value: Option<&str>) -> Result<(), String>;
}
struct CurrentUserStartup;

#[cfg(windows)]
impl StartupRegistration for CurrentUserStartup {
    fn read(&self) -> Result<Option<String>, String> {
        use windows_registry::CURRENT_USER;
        match CURRENT_USER
            .open(r"Software\Microsoft\Windows\CurrentVersion\Run")
            .and_then(|key| key.get_string("Fakkarni"))
        {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code().0 as u32 == 0x80070002 => Ok(None),
            Err(_) => Err("Could not read Windows startup registration.".into()),
        }
    }
    fn write(&self, value: Option<&str>) -> Result<(), String> {
        use windows_registry::CURRENT_USER;
        const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
        let result = if let Some(value) = value {
            CURRENT_USER
                .create(RUN)
                .and_then(|key| key.set_string("Fakkarni", value))
        } else {
            CURRENT_USER
                .options()
                .write()
                .open(RUN)
                .and_then(|key| key.remove_value("Fakkarni"))
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if value.is_none() && error.code().0 as u32 == 0x80070002 => Ok(()),
            Err(_) => Err("Could not change Windows startup registration.".into()),
        }
    }
}
#[cfg(not(windows))]
impl StartupRegistration for CurrentUserStartup {
    fn read(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn write(&self, value: Option<&str>) -> Result<(), String> {
        if value.is_none() {
            Ok(())
        } else {
            Err("Start with Windows is available only on Windows.".into())
        }
    }
}

fn startup_command(executable: &Path) -> Result<String, String> {
    let text = executable
        .to_str()
        .ok_or("Cannot register this executable path for startup.")?;
    if !executable.is_absolute() || text.contains(['"', '\r', '\n', '\0']) {
        return Err("Cannot register this executable path for startup.".into());
    }
    Ok(format!("\"{text}\" --autostart"))
}

fn save_settings(
    path: &Path,
    settings: &AppSettings,
    registry: &impl StartupRegistration,
    executable: &Path,
) -> Result<(), String> {
    let previous = registry.read()?;
    let desired = if settings.start_with_windows {
        Some(startup_command(executable)?)
    } else {
        None
    };
    let changed = previous != desired;
    if changed {
        registry.write(desired.as_deref())?;
    }
    if let Err(error) = persist_settings(path, settings) {
        if changed && registry.write(previous.as_deref()).is_err() {
            return Err(format!(
                "{error} Windows startup rollback also failed; check Windows Startup Apps."
            ));
        }
        return Err(error);
    }
    Ok(())
}

pub fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn lock(app: &tauri::AppHandle) {
    app.state::<Shared>().revoke();
    let _ = app.emit("vault-locked", ());
}

#[tauri::command]
pub fn get_app_settings(state: State<'_, LifecycleState>) -> Result<AppSettings, String> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "Application settings unavailable.")?
        .clone();
    // Windows may change startup registration outside the application.
    settings.start_with_windows = CurrentUserStartup
        .read()?
        .is_some_and(|value| !value.is_empty());
    Ok(settings)
}

#[tauri::command]
pub fn set_app_settings(
    state: State<'_, LifecycleState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let mut current = state
        .settings
        .lock()
        .map_err(|_| "Application settings unavailable.")?;
    let executable =
        std::env::current_exe().map_err(|_| "Could not locate the application executable.")?;
    save_settings(&state.path, &settings, &CurrentUserStartup, &executable)?;
    *current = settings.clone();
    Ok(settings)
}

/// Must run after the shared vault is managed, with main window `visible: false`.
pub fn init(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let path = app.path().app_local_data_dir()?.join("settings.json");
    // Preserve invalid files until an explicit settings save. Safe defaults must
    // never register startup implicitly or leave an inaccessible hidden window.
    let settings = read_settings(&path).unwrap_or_default();
    let minimized = settings.start_minimized;
    app.manage(LifecycleState {
        settings: Mutex::new(settings),
        path,
        quitting: AtomicBool::new(false),
    });

    let open = MenuItem::with_id(app, "open", "Open Fakkarni", true, None::<&str>)?;
    let lock_item = MenuItem::with_id(app, "lock", "Lock vault", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &lock_item, &quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("Fakkarni application icon is missing.")?;
    TrayIconBuilder::with_id("fakkarni-tray")
        .icon(icon)
        .tooltip("Fakkarni · Your passwords. Your PC.")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "lock" => lock(app),
            "quit" => {
                app.state::<LifecycleState>()
                    .quitting
                    .store(true, Ordering::SeqCst);
                lock(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    let window = app
        .get_webview_window("main")
        .ok_or("Fakkarni main window is missing.")?;
    let handle = app.handle().clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            let state = handle.state::<LifecycleState>();
            if state.quitting.load(Ordering::SeqCst) {
                return;
            }
            let close_to_tray = state
                .settings
                .lock()
                .map(|prefs| prefs.close_to_tray)
                .unwrap_or(false);
            if close_to_tray {
                api.prevent_close();
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            } else {
                state.quitting.store(true, Ordering::SeqCst);
                lock(&handle);
                handle.exit(0);
            }
        }
    });
    // `--autostart` identifies the launch source; visibility follows the user's
    // preference for both startup and ordinary launches.
    if !minimized {
        show_main(app.handle());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct FakeStartup(RefCell<Option<String>>, RefCell<Vec<Option<String>>>);
    impl StartupRegistration for FakeStartup {
        fn read(&self) -> Result<Option<String>, String> {
            Ok(self.0.borrow().clone())
        }
        fn write(&self, value: Option<&str>) -> Result<(), String> {
            let value = value.map(str::to_owned);
            self.1.borrow_mut().push(value.clone());
            *self.0.borrow_mut() = value;
            Ok(())
        }
    }
    #[test]
    fn failed_settings_save_restores_exact_startup_value() {
        let directory = tempfile::tempdir().unwrap();
        let old = Some("\"C:\\old location\\fakkarni.exe\" --autostart".to_owned());
        let registry = FakeStartup(RefCell::new(old.clone()), RefCell::new(Vec::new()));
        let settings = AppSettings {
            start_with_windows: true,
            ..AppSettings::default()
        };
        // Replacing an existing directory with a settings file must fail.
        let result = save_settings(
            directory.path(),
            &settings,
            &registry,
            &std::env::current_exe().unwrap(),
        );
        assert!(result.is_err());
        assert_eq!(*registry.0.borrow(), old);
        assert_eq!(registry.1.borrow().len(), 2);
    }
    #[test]
    fn startup_paths_are_quoted_and_opt_in_can_be_reversed() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory
            .path()
            .join("folder with spaces")
            .join("fakkarni.exe");
        let command = startup_command(&executable).unwrap();
        assert_eq!(command, format!("\"{}\" --autostart", executable.display()));
        assert!(startup_command(Path::new("relative.exe")).is_err());
        let registry = FakeStartup(RefCell::new(None), RefCell::new(Vec::new()));
        let path = directory.path().join("settings.json");
        let enabled = AppSettings {
            start_with_windows: true,
            ..AppSettings::default()
        };
        save_settings(&path, &enabled, &registry, &executable).unwrap();
        assert_eq!(*registry.0.borrow(), Some(command));
        save_settings(&path, &AppSettings::default(), &registry, &executable).unwrap();
        assert_eq!(*registry.0.borrow(), None);
        let calls = registry.1.borrow().len();
        save_settings(&path, &AppSettings::default(), &registry, &executable).unwrap();
        assert_eq!(registry.1.borrow().len(), calls);
    }
    #[test]
    fn first_run_is_visible_and_startup_is_opt_in() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            read_settings(&directory.path().join("settings.json")).unwrap(),
            AppSettings::default()
        );
        assert!(!AppSettings::default().start_with_windows);
        assert!(!AppSettings::default().start_minimized);
        assert!(AppSettings::default().close_to_tray);
    }
    #[test]
    fn preferences_roundtrip_and_atomic_replace() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        persist_settings(&path, &AppSettings::default()).unwrap();
        let changed = AppSettings {
            start_with_windows: true,
            close_to_tray: false,
            start_minimized: true,
        };
        persist_settings(&path, &changed).unwrap();
        assert_eq!(read_settings(&path).unwrap(), changed);
        assert!(String::from_utf8(std::fs::read(path).unwrap())
            .unwrap()
            .contains("startWithWindows"));
    }
    #[test]
    fn corrupt_unknown_and_oversized_preferences_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for invalid in [b"{".to_vec(), b"{}".to_vec(), b"{\"startWithWindows\":false,\"closeToTray\":true,\"startMinimized\":false,\"unknown\":true}".to_vec(), vec![b' '; MAX_SETTINGS_BYTES as usize + 1]] {
            std::fs::write(&path, invalid).unwrap();
            assert!(read_settings(&path).is_err());
        }
    }
}
