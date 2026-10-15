# Encrypted backups and PC transfer

Fakkarni exports its native encrypted `.kdbx` vault. A file can serve as a local backup or be carried to another Windows PC. There is no plaintext CSV export and no automatic upload or sync.

## Export on the old PC

1. Open and unlock Fakkarni.
2. Under **Backup & PC transfer**, choose **Export encrypted vault**.
3. Choose a new `.kdbx` filename in a local folder or removable drive. Existing files are refused rather than overwritten.
4. Keep the master password used for that snapshot. Windows Hello is tied to the old Windows user/device and is not a portable replacement for that password.

The exported file is an exact copy of the authenticated encrypted vault bytes. It contains the encrypted logins and notes, but no Windows Hello enrollment, browser account index, startup preferences, extension pairing, or native bridge registration. Export checks that the file still matches the unlocked session before copying it. If a Windows file dialog defaults to a synchronized folder, choose a different destination when you want the file to remain offline.

## Import on a new PC

1. Install or build Fakkarni on the new PC.
2. Choose **Import or restore an encrypted vault** from the locked/welcome screen, or **Import / restore vault** inside an unlocked vault.
3. Enter the exported file's master password and select the `.kdbx` file.
4. Review the filename and login count. Confirm that import replaces the local vault rather than merging entries, then choose **Import this vault**.
5. Unlock with the exported file's master password. Enable Windows Hello separately on the new PC and install/register the browser connection there.

Only Fakkarni's supported fixed KDBX profile is accepted in this milestone; this is not a general importer for arbitrary KeePass files or other password managers.

## Restore and recovery behavior

File selection, password validation, and preview do not replace the current vault. The preview expires after two minutes and is consumed once; locking or suspending revokes it. The source and current vault fingerprints are checked again at commit.

When a vault already exists, import first saves its exact encrypted bytes to a unique `fakkarni-before-restore-<id>.kdbx` file under the app's local `backups` directory. If that safety copy cannot be created, replacement stops. The confirmed candidate is then published atomically. Failure before publication leaves the old file and session intact; an already-created safety copy is preserved if the final replacement fails. A non-cooperating external writer's changes are not rolled back.

After a successful import the app stays locked, outstanding browser fills are cancelled, the optional locked-account index is cleared, and Windows Hello enrollment is disabled. Cleanup errors are reported as warnings alongside a successful restore, rather than claiming that the replacement did not happen. The result shows the safety-copy path when one was made. The old vault's master password is needed to open that recovery file.

The file chooser and all file operations run in native Rust; the frontend receives only filenames, counts, an expiring preview token, and result paths. No filesystem plugin access is granted to the frontend. Prepared restores retain encrypted snapshots, not a decrypted database or master key.

## Verification

Synthetic tests cover exact encrypted export, refusal to overwrite, wrong passwords, damaged and oversized files, excessive KDF settings, changed source/target files, existing instance locks, recovery-copy failures, forced Windows rename failure, missing-vault import, session revocation, expiry, and post-restore browser cleanup. Desktop editor regression tests cover ignored autofilled password fields and explicit password replacement. These tests have not reproduced the earlier username/password overwrite report, so its original cause remains unconfirmed.
