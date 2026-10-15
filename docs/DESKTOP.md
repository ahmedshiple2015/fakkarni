# Windows Hello and desktop preferences

Fakkarni remains a normal Windows desktop application. These features are configured inside the unlocked vault. Startup and Hello enrollment are off until the user explicitly enables them.

Open **Settings** in the sidebar after unlocking to configure Windows Hello, desktop preferences, browser autofill, and backup/PC transfer. **Passwords** returns to account search and details. The lock control is at the bottom of the sidebar. See [UI design](UI-DESIGN.md) for the shared desktop/extension theme and visual verification.

## Windows Hello

1. Create or unlock the vault using the master password.
2. Open **Settings**, choose **Enable Windows Hello**, and reenter that master password.
3. Complete Windows' own PIN, fingerprint, or face verification prompt.
4. Subsequent locked sessions show **Unlock with Windows Hello**, with master-password fallback always available.

Fakkarni never asks for, receives, or stores the Windows PIN or biometric data. The existing master password and KDBX encryption are unchanged. Keep the master password for recovery, a different PC, Windows/TPM reset, invalidated keys, and unsupported configurations. Hello enrollment persists across application restarts but is bound to this Windows user, device key, vault identity, and application vault path. Moving the vault or reinstalling/resetting Windows can require reenrollment.

Use **Disable Windows Hello** inside the unlocked vault to remove enrollment metadata and request deletion of this enrollment's OS key. If Windows key cleanup fails, the app reports that failure; the local enrollment file is still removed. Master-password access continues to work.

### Protection and compatibility

Enrollment version 2 explicitly uses RSAES-PKCS1-v1_5 for compatibility with Passport providers that rejected OAEP. This is legacy RSA encryption padding, not an OAEP or FIPS-compliance claim. Existing version-1 enrollment is never silently downgraded; disable it and enroll again. AES-GCM authenticates the encrypted credential and its binding. Only the application-owned enrollment is decrypted after mandatory fresh verification, with uniform errors; no browser or network API accepts arbitrary ciphertext.

The enrollment stores only encrypted data in `windows-hello.json` beside the vault in local application data. A random AES-256-GCM key encrypts the master credential with authenticated version/scheme/vault/path/key-name binding. The public portion of an RSA key in the Microsoft Passport Key Storage Provider is exported as an RSA public blob and used with BCrypt to wrap that symmetric key using RSAES-PKCS1-v1_5. Only public key material leaves the provider; private-key decryption stays in Passport. Enrollment must successfully unwrap the key through Hello before any enrollment file is saved. Decryption requires a non-exportable TPM-backed key with mandatory fresh user verification. Secrets remain in the native Rust code and are cleared where supported; neither the browser extension nor the frontend receives the recovered master credential.

The implementation checks the expected current-user enrollment name, key size, permitted usage, export policy, hardware implementation, and mandatory-gesture policy. Unsupported properties, software keys, unsupported RSA padding, cancellation, and key errors fail closed. There is no DPAPI-only or cosmetic verification fallback.

`NgcCacheType`, `NgcKeyImplType`, and `PinCacheIsGestureRequired` are provider-specific properties without a complete public Microsoft compatibility contract. Their support must be verified on target Windows builds. Automated tests cover mock encryption, tampering, wrong keys, metadata bounds, failed writes, identity binding, and revoked requests; they do not prove that a real device presents a fresh Hello prompt. Manual hardware verification remains required before treating this feature as production-ready.

Enrollment reauthenticates against the current encrypted vault before any key is created. Lock/suspend/quit events revoke pending work, and a completed Hello prompt cannot unlock after its request was revoked. Browser requests cannot submit ciphertext to Hello or trigger decryption directly. Browser credential delivery requires an unlocked vault and an account selection in the extension; selecting while locked opens the desktop unlock flow and fills afterwards.

References: [NCryptDecrypt](https://learn.microsoft.com/en-us/windows/win32/api/ncrypt/nf-ncrypt-ncryptdecrypt), [Passport KSP and Windows Hello architecture](https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/rdp-sign-in), [CNG key properties](https://learn.microsoft.com/en-us/windows/win32/seccng/key-storage-property-identifiers), and [KeePassWinHello's independently implemented integration](https://github.com/sirAndros/KeePassWinHello/blob/master/src/AuthProviders/WinHelloProvider.cs).

## Tray and startup

- **Start with Windows:** opt-in registration for the current Windows user only. The executable's full path is quoted and registered with `--autostart`. No administrator privileges or system-wide entry are used.
- **Keep running in system tray when I close the window:** enabled by default. Clicking X hides the window while Brave can continue making queries. Existing five-minute inactivity locking and Windows lock/suspend locking continue to apply.
- **Open in system tray:** off by default. When enabled, both normal launches and Windows startup begin with the window hidden and a tray icon available.
- **Tray menu:** Open Fakkarni, Lock vault, Quit. Left-clicking the icon opens the window. Quit locks and exits the app completely.
- **Opening a second copy:** restores the existing window instead of starting another vault process. A duplicate automatic-start launch does not force the existing window into the foreground.

Startup never unlocks the vault automatically. The browser can start Fakkarni in the background. Password filling requires vault unlock and an account selection in the extension. Account suggestions while locked require the separate local-index opt-in. Use Hello or the master password after each application start.

Settings are bounded JSON written atomically in local application data. A failed settings save attempts to restore the exact previous startup registration. Tests use a fake registry and never enable actual startup. This prototype has no installer: enabling startup points to the current executable, so keep that executable in place until an installer provides a stable installation path.

Private Hello operations open the Passport key with UI allowed, while public-key export remains silent. Opening the private key with NCRYPT_SILENT_FLAG causes NTE_SILENT_CONTEXT (0x80090022) when verification is needed; it must not be used for interactive unlock. TPM and fresh-gesture checks remain mandatory.

## Editing saved passwords

Ordinary title, username, site, and notes edits preserve the existing protected password. The desktop editor receives no existing password and keeps replacement controls disabled until **Change password** is selected. New and replacement passwords require confirmation. The native save command defaults to preserving an existing password when no replacement flag is supplied, and the vault core skips the protected Password field entirely in that case. General WebView2 form autofill is disabled inside the desktop app.

These safeguards address accidental editor overwrites; they do not establish the cause of an earlier report where the revealed password matched the username. Synthetic read/fill/save/reopen regression tests must continue to check that username and password stay distinct and read-only operations leave encrypted vault bytes unchanged. No existing user password is repaired or guessed automatically.

## Backup and PC transfer

Use **Export encrypted vault** to create a portable `.kdbx` file and **Import / restore vault** to load it on this or another PC. A preview and explicit replacement confirmation come before any vault change. The current encrypted vault is preserved before replacement, and the restored vault requires unlocking with the export's master password. See [backup and transfer instructions](BACKUPS.md).

## Deleting a login

Select a login, choose **Delete login**, and confirm the account shown. The current encrypted vault is updated atomically; a failed write preserves the prior entry. Deletion also removes cached browser suggestions and cancels a pending fill for that login. Exported backups retain their prior copies; deleting a login is not a secure-erasure operation on older backups or website forms that already received it.
