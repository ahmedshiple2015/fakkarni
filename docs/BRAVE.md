# Brave and Edge autofill

Fakkarni is a Windows desktop app with an optional Chromium extension. The extension lists matching accounts and fills one login when the vault is unlocked, or automatically after unlocking. It does not save logins automatically or submit forms.

## Setup

Build the desktop app and native host, then register the browser connection:

```powershell
npm ci
npm run desktop:build -- --debug
cargo build -p fakkarni-native-host
node extension/build.mjs
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/register-brave.ps1
# For Edge instead, or as well:
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/register-edge.ps1
```

Load this repository's `extension` directory at `brave://extensions` or `edge://extensions` with Developer mode enabled. Its fixed ID is `kieggakkiiegglikoiojkbfabpmmahep`. Reload after updating; this version adds the `scripting` permission. Fully exit the browser after registration if it reports HOST_NOT_FOUND: Edge Startup Boost can keep its background process alive after its window closes. Do not end browser processes before saving unfinished work.

Registration copies the native host to local application data, writes a bounded desktop executable path configuration, and registers it for the current Windows user. Keep the desktop executable in place; pass `-DesktopPath` when registering a different installed location. Updating the native executable requires registering again. Browser requests may start Fakkarni hidden in the tray; selecting an account brings it forward. Remove registrations with the corresponding unregister script; vault data is preserved.

## Fill a login

1. Save a synthetic login in Fakkarni with the site's HTTPS address.
2. Open the extension on that site and choose the account.
3. If the vault is locked, Fakkarni comes forward with the account and exact site. Unlock using the master password or configured Windows Hello. Filling resumes automatically. If already unlocked, the browser fills immediately without bringing the desktop forward.
4. The extension checks the original tab and document again, fills the login, and returns focus to the browser. Review and submit the form yourself.

Requests expire after two minutes. Navigation, reload, closing the tab, cancellation, or locking before delivery prevents delivery. Password-only steps, ambiguous forms, signup/password-change pages, and iframes are deliberately unsupported. Reopen the extension after an exclamation badge to retry or fill manually. Browsers do not generally allow the toolbar popup to open itself on arbitrary pages; a toolbar click starts this flow.

## Accounts while locked

Inside the unlocked desktop app, enable **Show saved accounts in the browser while my vault is locked**. This is off by default. It writes a separate unencrypted local index of account names, usernames, and HTTPS origins. Programs running as your Windows user can read that index. Passwords and notes stay in the encrypted vault. Turning the setting off removes the cached entries. The index refreshes while the vault is unlocked without extending its idle timeout; it is bounded to 512 entries and 512 KiB.

## Security boundaries

- Permissions are `activeTab`, `nativeMessaging`, and `scripting`; no broad host permissions, persistent extension account storage, network calls, or always-running page scripts.
- Exact HTTPS origins include ports; subdomains and lookalikes do not match. Full URL and main-frame document identity are checked before injection, blocking same-URL reloads as well as navigation.
- Normal matching returns only bounded account metadata. Passwords are delivered while the vault is unlocked, using an expiring random capability bound to one account and origin, and consumed once. Selecting an account in the extension authorizes filling without an additional desktop approval. They never enter the popup or desktop frontend during this handoff.
- The protected per-user named pipe rejects remote clients but cannot authenticate against another program running as the same Windows user. With automatic filling enabled by this design, another program running as the same Windows user can request credentials from the unlocked vault, or queue a request to be fulfilled after unlocking. Native-host caller checks alone do not authenticate browser intent. This convenience flow trusts the Windows user account; it does not claim protection against malicious programs under that account or a compromised operating system.
- Lock state and epoch are rechecked before credential delivery. Rust clears credential buffers where supported; JavaScript and browser internals cannot promise complete immediate erasure.
- Filling intentionally gives the credential to the selected website, whose own scripts can read its fields. No form is submitted automatically.

## Validation

Run `npm run test:extension`, `cargo test --workspace --locked`, and strict Clippy. Synthetic tests cover immediate and post-unlock delivery, repeated fills, one-time delivery, origin binding, cancellation, lock revocation, metadata limits, navigation/document changes, and unsupported forms. A process smoke test verifies cold desktop launch and closure of the native protocol pipe. Real browser filling and Windows Hello prompts still require manual checks on target devices.

References: [Chromium Native Messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging), [Microsoft Edge Native Messaging](https://learn.microsoft.com/en-us/microsoft-edge/extensions/developer-guide/native-messaging).

## Bridge troubleshooting

Run `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-browser-bridge.ps1 -Probe` to check both registry views, the installed manifest and executable, desktop launch configuration, and a synthetic-origin native protocol request. It prints only booleans, protocol status, and a match count; it does not print stored accounts, passwords, or response bodies. A passing direct probe does not establish that an already-running browser has rediscovered its native host.

Registration is idempotent: unchanged binaries, JSON files, and registry values are left untouched. Changed files are staged, flushed, and replaced atomically; Brave and Edge registration is serialized because they share the same host. This prevents partial installed files during updates. If the direct probe passes but the browser still reports HOST_NOT_FOUND, save unfinished work and fully exit that browser, including background processes, before relaunching. No vault reset or recreation is needed.
