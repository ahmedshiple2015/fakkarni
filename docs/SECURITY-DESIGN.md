# Security design requirements

This document records requirements, not verified security guarantees.

## Privacy boundary

- Runtime vault features work offline and make no outbound service calls.
- Bundle fonts, icons, scripts, and styles locally; never fetch site favicons.
- Do not log passwords, keys, login candidates, or sensitive vault metadata.
- Store encrypted vaults and backups outside automatically synchronized folders by default; users control their backup destination.
- Build-time dependency downloads and optional browser-store distribution are separate from runtime vault behavior and must be documented.

## Threat model

Protect encrypted data on disk and reduce accidental disclosure to webpages or local processes. An attacker controlling the operating system or an unlocked application can defeat these boundaries. User-selected filling exposes the credential to the destination page/application.

## Vault and session

- Prefer reviewed, interoperable KDBX implementations; do not invent cryptography.
- Use Argon2id when supported by the selected KDBX configuration and benchmark parameters on target machines.
- Keep secrets in the Rust core where practical; send only what an explicit UI operation requires.
- Minimize secret lifetime and copies; use memory clearing where supported without promising complete removal from all memory.
- Lock on Windows lock, suspend, configured inactivity, and explicit user action. Revoke extension sessions and outstanding requests on lock.
- There is no email-based master-password recovery. Explain backup and password-loss consequences during onboarding.
- Persist atomically, preserve prior valid data on write failure, and test restore flows.

## Browser boundary

- Allow only the installed extension's authorized identity in the native host registration; this does not protect against a compromised local account.
- Treat page content and content-script messages as untrusted. Validate message schema, lengths, operations, and session authorization.
- Obtain tab/frame context from browser APIs rather than trusting a page-supplied hostname.
- Default to exact normalized hostname matching. Additional host associations require deliberate user configuration.
- Validate scheme, target frame origin, tab/document identity, and navigation state immediately before filling. Reject insecure HTTP filling by default.
- Cross-origin frames need their own explicit matching rules; never inherit permission solely from the top-level page.
- Return matching summaries only as necessary and retrieve one selected credential after a user gesture. Do not expose vault-wide retrieval to the extension.
- Do not automatically submit forms.
- Detecting form submission does not prove authentication succeeded. Save/update prompts require user confirmation.
- Discard unsaved candidates promptly, including when locking. Do not persist plaintext candidates in extension storage.

## Import and clipboard

- Treat imported files as untrusted and show a preview before vault mutation.
- Browser CSV exports may be plaintext; avoid creating intermediate plaintext copies and explain source-file handling during import.
- Clipboard copying is explicit. Clear only if the clipboard still contains the application's copied value; clipboard history and third-party tools may retain copies.

## Required review evidence

Interoperability, corrupted input handling, interrupted writes, locked-session refusal, origin matching, navigation races, malicious frames, IPC authorization, and absence of unexpected runtime network requests must be verified before release.

## Current autofill implementation

Selecting an account in the extension fills it immediately while unlocked. If locked, the desktop comes forward for master-password or Windows Hello unlock, then filling resumes without another approval. The protocol binds an expiring one-time request capability to the exact origin/account, checks lock epochs before release, and checks navigation/document identity before injection.

The local pipe is not proof of browser identity. Removing desktop approval intentionally trusts programs running as the current Windows user: those programs can request a credential while unlocked, or queue a fill that completes after unlock. A random request ID protects consumption/replay but is not authentication of the requesting executable. This prototype does not claim protection from malicious programs under the current Windows account. See [browser setup and boundaries](BRAVE.md).

The optional locked-account index exposes account metadata locally after explicit opt-in, while preserving password encryption.
