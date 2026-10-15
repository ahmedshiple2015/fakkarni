# فكرني · Fakkarni

A local-only, open-source desktop password manager for Windows. It runs as a normal desktop program; the Microsoft Store is not required.

**Status:** early desktop prototype, not a production release. Use synthetic credentials while storage, recovery, Windows integration, and interoperability are being validated. The KDBX writing library is experimental; see [the library decision](docs/LIBRARY-DECISION.md).

## Implemented in the prototype

- Rust vault core with encrypted KDBX 4.1 storage and Argon2id (64 MiB, three passes).
- Create, unlock, lock, add/edit logins, and secure random password generation.
- Dark Tauri desktop interface with local search and explicit, temporary password reveal.
- Five-minute inactivity locking and Windows session/power event handling.
- Encrypted temporary writes, atomic replacement, cooperating-instance locking, and external-change detection.
- Restricted file profile checked before key derivation. General-purpose KDBX import is not supported yet.
- Brave extension preview for matching account names/usernames, connected through a local Windows pipe and Native Messaging.
- Optional Windows Hello quick unlock, keeping the master password as recovery fallback; requires compatible TPM-backed Windows Hello protection.
- System tray with Open, Lock vault, and Quit; optional startup with Windows and opening in the tray.

The extension currently shows suggestions only. Automatic saving/filling, password import UI, backup/restore UI, and native-app detection are still pending. See [Brave setup](docs/BRAVE.md) and [Windows Hello and desktop preferences](docs/DESKTOP.md).

## Run locally

Install Node.js 24 LTS, stable Rust (MSVC), Visual Studio C++ build tools with Windows SDK, and WebView2. Then:

```powershell
npm ci
npm run desktop
```

For the workspace-local npm bootstrap, use `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/npm.ps1 run desktop`.

The native app stores `vault.kdbx` under the current user's local application data directory for `org.fakkarni.desktop`. It has no account or cloud connection. `npm run dev` opens a design preview with credential inputs disabled; it cannot store passwords.

See [contributing and checks](CONTRIBUTING.md) for build and test instructions.

## Agreed first release

- Windows 10/11 desktop application and Brave extension.
- Master-password unlock; automatic lock when Windows locks.
- Encrypted local vault, preferably KDBX after library evaluation.
- Create, edit, search, and generate passwords.
- Save/update prompts for browser logins and explicit click-to-fill.
- Password import and encrypted local backup/restore.
- Clean dark interface with Arabic and English naming.
- GPL-3.0-only license, with the standard license text included.

No cloud service, telemetry, remote assets, or automatic network update checks. Filling credentials intentionally gives them to the selected website or application during login.

## Later milestones

Windows Hello unlock, Firefox support, foreground-app suggestions, and individually tested filling for desktop applications such as Discord, Steam, and Riot clients.

See [the implementation plan](docs/PLAN.md) and [security design](docs/SECURITY-DESIGN.md).
