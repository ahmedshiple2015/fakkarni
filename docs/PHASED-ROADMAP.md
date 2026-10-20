# Fakkarni implementation and acceptance plan

Each phase passes through implementation, automated verification, and user testing.
Only the user's successful test makes a phase accepted. Stop between phases to
resolve failures before starting the next. Tests use synthetic vaults and credentials.

## Phase 1 — Change a known master password

Status: automated verification passed; awaiting user testing, not yet accepted.

- Require an unlocked vault and verify the current master password again.
- Require a different new password of at least 12 characters, with confirmation.
- Save an exact encrypted safety copy before atomically publishing a verified
  replacement. Preserve entries, passwords, notes, and vault identity.
- Lock after success, invalidate pending browser operations, reset locked-account
  suggestions, and remove the old PIN enrollment. Set up PIN again after unlocking.
- Report cleanup failures separately from a successful password change.
- Existing backups and the safety copy still need their original master password.

Automated gate: successful rotation and data preservation; wrong password; invalid
new password; failed backup; failed publication; concurrent file replacement;
revoked requests; native cleanup; dialog validation and lock-event handling.

User acceptance checklist:

1. Export an encrypted backup first and retain its current master password.
2. Open Settings → Master password → Change master password.
3. Try a wrong current password and mismatching confirmation. Neither should change
   the vault. Cancel should leave it usable.
4. Submit the correct current password and a different confirmed new password.
   Expect a success message with an encrypted safety-copy path and a locked vault.
5. Confirm the old password fails and the new password unlocks. Check existing
   login details remain unchanged without sharing any passwords in this chat.
6. Set up PIN again, lock, and test PIN unlock. Restart and confirm the new master
   password works. Previously exported backups retain their original password.

Verification record (2026-10-15): `cargo test --workspace --locked` passed 71
tests with one intentionally ignored fixture generator. Strict workspace Clippy
passed. Frontend build and seven desktop form tests passed. Disposable-browser
checks passed for dialog validation, cleared input fields, cancellation while
saving, authentication failures, and success reporting after an early lock event.
Existing navigation, editor, responsive-layout and extension-popup checks passed.
The Windows desktop build passed. These checks used synthetic data; user acceptance
and a real Windows PIN re-enrollment test remain outstanding.

## Phase 2 — Forgotten master password while PIN still works

Status: planned; requires Phase 1 acceptance.

Add a separate recovery action requiring a fresh Windows Hello/PIN verification,
even if the vault is already unlocked. Use the recovered credential only in native
code to rotate the password through Phase 1's verified save path. Never return the
old password to the frontend. Bind authorization to this vault, one operation, and
the current session; cancellation, Windows locking, or a changed vault revokes it.

This requires PIN enrollment already working on that PC. Configure enrollment once
per PC; there is no universal PIN or per-PC code customization. Retain the master
password route for unsupported hardware or unavailable Windows Hello.

Automated gate: stale and mismatched enrollment, cancelled verification, lock races,
repeat requests, failed saves, and successful rotation with preserved entries.
User gate: forget-password simulation on a test vault, fresh PIN prompt, cancel,
successful replacement, new-password unlock, and renewed PIN setup.

## Phase 3 — Optional offline recovery key

Status: planned; requires Phase 2 acceptance and a reviewed storage design.

Generate a cryptographically random recovery key locally. Show/export it only on
explicit setup and require confirmation that it was saved separately. Use an
authenticated encrypted recovery envelope bound to the vault and its version;
never store a plaintext master password. Define key rotation, import/export, and
password-change behavior before implementing the format. Recovery must work on a
second PC without depending on that PC's Windows PIN.

Explain the limits: losing both the master password and recovery methods means
losing access; anyone with a recovery key and its matching encrypted files can
unlock them. Disabling a key cannot revoke copies of older backups already held.

Automated gate: wrong key, tampered/truncated files, vault mismatch, version checks,
rotation, rollback/failure handling, and portable recovery with synthetic files.
User gate: restore on a separate test location, recover, choose a new master
password, and confirm entries and new PIN enrollment.

## Phase 4 — Offer to save a manually entered login

Status: queued after recovery phases are accepted.

Recognize username/email and password fields, keep proposed credentials briefly
in memory, and ask before saving. Avoid ambiguous forms and discard expired or
cancelled candidates. Detecting form submission alone does not prove login success.
Do not silently replace any existing password.

Test ordinary, email-only, username-only, revealed-password, multi-step, cancelled,
failed-login, and navigation cases. User acceptance on Brave and Edge is required.

## Phase 5 — Registration and password-update prompts

Status: queued after Phase 4 acceptance.

Recognize new-password and confirmation fields, offer explicit save/update actions,
and show which existing account would change. Keep password replacement deliberate
and preserve the existing metadata-only edit safeguard. Test mismatched confirmation,
multiple accounts, generated passwords, failed registration, and repeated autofill.
User acceptance includes checking the stored password stays correct afterward.

## Phase 6 — Installation and release hardening

Status: queued.

Package an installer with stable executable paths and Brave/Edge bridge registration;
verify upgrade, uninstall, startup/tray, and clean-PC behavior. Perform a focused
security review before treating the app as ready for important credentials.

Follow-up from cold-start autofill verification (2026-10-20): investigate an
intermittent EOF in the parallel named-pipe integration test. Its isolated and
full serial reruns passed. Include explicit extension-worker reload/update checks
in upgrade testing so popup and background code cannot be mistaken for a broken
native registration.
