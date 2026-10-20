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
- Vault locked: matching accounts show **Unlock & fill**. Choosing one brings the
  desktop app forward, then resumes filling after password or PIN unlock.
- Desktop fully closed: the bridge reads the optional local account index without
  opening the app. Choosing **Unlock & fill** starts Fakkarni, brings it forward,
  and waits for the same unlock flow. Merely opening the popup does not start it.
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
4. Choose an account, unlock with PIN or the master password, and check the form
   fills automatically. The unlocked popup should show **Fill** instead.
5. Quit Fakkarni completely from its tray menu. Open the extension: accounts should
   still appear and the desktop app should stay closed until an account is chosen.
6. Choose an account and confirm the app opens, requests unlock, and fills afterward.
7. Restart Windows and repeat the closed-app test in Brave and Edge.

Do not share any actual passwords while reporting test results.
