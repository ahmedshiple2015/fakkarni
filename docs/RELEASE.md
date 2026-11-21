# Fakkarni 0.3.3 release candidate

Run `scripts/build-release.ps1` on Windows x64 with the prerequisites in the README and npm dependencies installed. The installer, unpacked extension ZIP, and SHA-256 checksums appear in `artifacts/release`.

1. Run the per-user installer from your normal Windows account. Quit Fakkarni, Brave, and Edge first when upgrading.
2. In Brave or Edge, open the extensions page, enable Developer mode, and load the installed `browser-extension` folder. For an existing unpacked extension, reload it if it already points to the installed folder. Otherwise replace that copy with the one from the installed folder.
3. Open Fakkarni and unlock a disposable vault. Test a login on a trusted site. If the bridge needs repair, use **Repair browser bridge** in Fakkarni, then fully restart the browser, including background processes.

The installer registers the bridge for your Windows user and migrates an existing Fakkarni startup opt-in to the installed executable. The default uninstall preserves vault data and removes bridge and startup registrations owned by this installation.

The extension needs access to HTTPS pages to detect submissions and notification permission to offer a save. After you submit a recognized same-origin POST form, choose **Review login**. Fakkarni opens if it was closed and asks you to unlock if needed. Approve the save once the vault is unlocked. If Windows suppresses notifications, look for the **Save** badge in the extension. A candidate expires after 25 seconds in the browser or 90 seconds after reaching the desktop app. Confirm the site accepted the login, registration, or password change before saving.

The installer is unsigned. The release checks use synthetic data. Real Windows Hello enrollment and browser focus depend on Windows hardware and desktop state and need a separate interactive check. The storage library and release have no independent security audit.
