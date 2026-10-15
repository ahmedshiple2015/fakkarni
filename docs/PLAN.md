# Implementation plan

The current phase order and user acceptance gates are tracked in
[PHASED-ROADMAP.md](PHASED-ROADMAP.md). That roadmap takes precedence over
the original milestone order below.

## Architecture

1. Rust core owns vault operations, unlocking, locking, and sensitive state.
2. Tauri desktop shell provides a bundled local TypeScript interface.
3. A TypeScript Manifest V3 extension supports Brave first.
4. A registered Native Messaging host connects the extension to the desktop core. If a separate desktop process is involved, use access-controlled local IPC with a defined authentication and session protocol.
5. The extension keeps preferences, not persistent credentials or vault keys.

The Native Messaging installer must be validated specifically in Brave. Browser compatibility alone does not establish correct host registration.

## Milestone 1: vault foundation

- Install/verify Rust, Cargo, Windows build prerequisites, and JavaScript package tooling.
- Evaluate maintained Rust KDBX libraries for license compatibility, KDBX version support, authenticated integrity, Argon2id support, and round-trip compatibility with KeePassXC.
- Record the selected library and threat model before implementing storage.
- Create the desktop shell and core with a narrow command interface.
- Implement create/unlock/lock, entry editing, secure generation, and atomic persistence.
- Verify wrong-password rejection, corruption handling, lock behavior, backup restoration, and round-trip interoperability.

Acceptance: create a vault, add a synthetic login, close/reopen it, unlock, and recover the login; open a compatible test vault in KeePassXC. Verify failures preserve the last valid file.

## Milestone 2: Brave integration

- Implement installer registration and extension authorization.
- Define bounded requests for session state, matching summaries, and explicit credential retrieval.
- Implement exact-host matching and extension account chooser.
- Bind each fill to its original tab, frame, document, and origin; revalidate before disclosure.
- Implement save/update prompts with confirmation and short-lived in-memory candidates.
- Test ordinary, multi-step, dynamically rendered, and cross-origin-frame login forms.

Acceptance: save a synthetic login, suggest it on the intended host, fill on user action, reject unrelated hosts and stale requests, and refuse credential access while locked.

## Milestone 3: usable first release

- Add import preview, duplicate handling, encrypted backups, settings, and recovery guidance.
- Add inactivity/suspend locking and carefully bounded clipboard behavior.
- Document permissions and local storage locations.
- Include GPL-3.0 license text, third-party notices, build instructions, and a vulnerability reporting policy.
- Review dependencies and security-sensitive code before real-password use.

## Milestone 4: desktop integrations

- Offer a global shortcut and explicit entry-to-application associations.
- Identify foreground executables using path and publisher information where available.
- Test UI Automation support for each application's login screen.
- Keep manual copy available; only claim filling support after application-specific testing.
- Avoid process injection, game hooks, and elevation for ordinary filling.

## Deferred

Windows Hello, additional browsers, inline website suggestions, and broader desktop filling. No schedule is committed until the vault-library and toolchain checks are complete.
