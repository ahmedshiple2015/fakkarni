# Accounts before unlocking

## Setup once

Unlock Fakkarni, open Settings, and enable **Show accounts while locked**.
This builds a local account index containing only entry identifiers, account
names, usernames and HTTPS website origins. Passwords and notes remain in the
encrypted vault. The index is outside that encryption and readable by programs
running as the Windows user. Disable the setting to remove its account metadata.

An existing encrypted vault must be unlocked once to build this list. It cannot
be reconstructed from encrypted data without a valid unlock method. Once enabled,
the list persists across application restarts and Windows restarts; a first unlock
after every reboot is not required. Import/restore and master-password changes
reset this setting, so enable it again after those operations.

## Extension behavior

- Vault unlocked: matching accounts show **Fill**. Choosing one fills the current
  page without another desktop approval prompt.
- Vault locked with PIN enabled: matching accounts show **Unlock & fill**. Choosing
  one opens the Windows verification prompt over the browser without bringing the
  vault window forward. Successful verification unlocks and fills automatically.
  Cancelling verification cancels the fill without opening the vault window.
- Vault locked without PIN, or with unusable PIN setup: choosing **Unlock & fill**
  brings the master-password screen forward, then fills after unlocking.
- Desktop fully closed: the bridge reads the optional local account index without
  opening the app. Choosing **Unlock & fill** starts Fakkarni in the background,
  then uses the same direct PIN prompt or master-password fallback. Merely opening
  the popup does not start it.
- No enabled index: the extension explains the one-time setup. No passwords or
  account names are guessed or decrypted while locked.

Matching stays restricted to the exact HTTPS origin. Cached summaries never
authorize password disclosure; the desktop rechecks the actual unlocked vault,
entry and origin. Filling remains bound to the selected tab and document and
does not submit the form. Cancelled, expired or navigated requests do not fill.
The extension does not persist passwords or duplicate the account list in browser
storage. The installed bridge must be available for this feature to work.

## Verification and user acceptance

Automated checks cover the shared bounded metadata parser, unknown/secret fields,
disabled and malformed indexes, exact-origin cached matches, response-size limits,
no desktop launch for listing accounts, launch-on-selection, caller validation,
and the locked/unlocked labels and repeated-click handoff.

User acceptance is pending:

1. Run `scripts/repair-browser-bridge.cmd` from File Explorer to install the updated
   native host in the normal Windows session. Reload the extension in both browsers.
2. Launch Fakkarni normally, unlock once, and enable the setting above.
3. Lock the vault. The popup should show accounts with **Unlock & fill**.
4. With PIN enabled, choose an account and confirm only Windows verification
   appears over the browser. Complete it and check the form fills automatically.
   The vault window should not be brought forward. The unlocked popup should show
   **Fill** instead. Lock again and cancel PIN: neither filling nor the vault
   window should appear. Try again to confirm cancellation permits a fresh request.
5. Quit Fakkarni completely from its tray menu. Open the extension: accounts should
   still appear and the desktop app should stay closed until an account is chosen.
6. Choose an account and confirm the app starts in the background, Windows PIN
   appears directly, and filling follows without bringing the vault forward.
7. Restart Windows and repeat the closed-app test in Brave and Edge.
8. With PIN disabled on a test setup, confirm the master-password screen still
   appears and filling resumes after password unlock.

Do not share any actual passwords while reporting test results.

## Native prompt ownership

The desktop captures the foreground root-owner window and process identifier on
the selected account request, verifies that it belongs to a supported browser
process, and checks it again before prompting. This avoids
using the transient extension popup as an owner. Windows creates the verification
UI; Fakkarni never renders or receives the Windows PIN. Successful verification is
bound to the current vault, session epoch and still-pending browser request.
No foreground permission is granted to arbitrary processes.

The generic CNG property supports a parent HWND for provider UI; cross-process
window ownership is supported by Windows. The specific provider's focus behavior
still needs the manual browser test above.
[CNG key properties](https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/SecCNG/key-storage-property-identifiers.md),
[Windows cross-process ownership](https://devblogs.microsoft.com/oldnewthing/20110331-00/?p=11083).

Direct-PIN verification record (2026-10-20): 88 Rust tests passed across the
workspace and final desktop rerun; one fixture generator remains intentionally
ignored. Strict workspace Clippy, seven desktop form tests, 23 extension tests,
the frontend and desktop builds, and the synthetic native-event UI check passed.
Tests never opened a real vault or PIN prompt. Actual Windows prompt placement,
cold-start behavior, cancellation, and fallback remain awaiting user acceptance.
