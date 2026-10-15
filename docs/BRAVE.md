# Brave account suggestions preview

Fakkarni is a regular Windows desktop application. The optional extension connects Brave to the already-running desktop vault; it does not replace that application.

This milestone shows matching account names and usernames. It deliberately has no API for password retrieval, filling, saving forms, or unlocking the vault. These operations need their own security and usability work before being enabled.

## Developer setup

From the repository root, after installing the prerequisites in CONTRIBUTING.md:

```powershell
npm ci
npm run build:extension
cargo build -p fakkarni-native-host
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/register-brave.ps1
npm run desktop
```

For this workspace's local npm bootstrap, replace `npm` with `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/npm.ps1`; `run build:bridge` invokes Cargo with the local toolchain on PATH. The process-only execution-policy flag does not change the machine's policy.

In Brave, open `brave://extensions`, enable Developer mode, choose **Load unpacked**, and select this repository's `extension` directory. The expected ID is `kieggakkiiegglikoiojkbfabpmmahep`. Open and unlock Fakkarni, save a synthetic login with a full HTTPS website address, then visit that site and click the extension. The popup offers **Refresh accounts** after changes.

Registration copies the compiled native host into `%LOCALAPPDATA%\org.fakkarni.desktop\native-messaging` and adds only this host under the current user's Brave NativeMessagingHosts registry key. It does not install an extension or modify Brave's built-in password manager. The script has been prepared for review; loading the unpacked extension is a manual step. Native executable updates require rerunning registration to copy the updated host.

To disconnect, disable/remove the extension and run `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/unregister-brave.ps1`. This removes only the host registration, preserving vault data and the installed host files.

## Privacy and matching

- The extension has only `activeTab` and `nativeMessaging` permissions. It checks the active page when its popup is opened or refreshed. There are no content scripts, background browsing collection, network calls, or persistent account caches.
- Only the active top-level canonical HTTPS origin crosses to the desktop, without its path, query, fragment, form contents, or typed credentials.
- Exact origin matching includes the port. Subdomains are separate: `login.example.com` does not automatically match `example.com`. Similar-looking and suffix-based phishing domains are not matched.
- The desktop returns a bounded list of IDs, names, usernames, and the requested origin. It returns no passwords, notes, or stored URL paths.
- The vault must already be unlocked. Browser queries do not refresh its inactivity timer. Locked, suspended, or busy desktop states do not disclose suggestions.
- The desktop connection uses a user-specific Windows named pipe with a protected ACL for the current user and SYSTEM, rejects remote clients, and applies frame-size and I/O-time limits. It does not open a TCP/HTTP listener.
- Native host registration allows only the expected extension origin; the executable also verifies its browser-supplied caller argument. These checks do not authenticate against a malicious process that already runs as the same Windows user. Such processes can request the metadata-only pipe. There is no cryptographic extension pairing in this prototype, and this protocol must not be extended to release credentials without a separate authorization design.
- Suggestions already delivered to an open popup cannot be recalled instantly on desktop lock. They disappear when the popup closes or refreshes. No password is involved in this milestone.

## Validation

Run `npm run test:extension` and `cargo test --workspace --locked`. Tests cover canonical origins, phishing lookalikes, timeouts, invalid framing/operations, native caller identity, lock refusal, and matching metadata. On 2026-10-15, 23 Rust tests and 6 extension tests passed, along with strict Clippy and desktop/native-host builds. A process smoke check confirmed that the compiled native host reached the running desktop app and received a locked response with no metadata. Real Brave loading and end-to-end browser behavior still need manual verification; compile and protocol tests alone do not establish that integration.

Native Messaging references: [Chromium protocol](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging) and [Bitwarden's Brave host-registration implementation](https://github.com/bitwarden/clients/blob/main/apps/desktop/src/main/native-messaging.main.ts).
