# Browser bridge installation context

If Brave or Edge reports `HOST_NOT_FOUND`, check the registration from the same
normal Windows desktop session that launches the browser. A successful check in
a development or isolated execution environment does not prove that an ordinary
desktop browser can see those registry keys or files.

On 2026-10-20, the development-session check saw the Brave, Chrome, and Edge keys
and the installed bridge files. A check launched by the user in File Explorer
saw no Brave or Edge keys and no installed manifest or desktop configuration.
The Chrome fallback key existed but pointed to a manifest that was absent there.
This explains why browsers launched from the development session worked while
the user's normal launches failed. The underlying isolation mechanism has not
been identified; the differing installation visibility was directly measured.

Running the repair from the desktop then exposed a concrete installer defect:
`New-Item` failed when `NativeMessagingHosts` or its browser ancestors did not
exist. Tests in an already configured environment had missed this clean-install
case. Registration now uses `RegistryKey.CreateSubKey` to create the complete
parent chain and verify the default `REG_SZ` value in both registry views.

## Repair in the normal Windows session

1. Build the desktop and native-host executables if they do not exist yet.
2. In File Explorer, double-click `scripts/repair-browser-bridge.cmd` as the normal
   Windows user. Do not use Run as administrator or an isolated development shell.
3. The launcher runs the existing atomic per-user registration scripts for Brave
   and Edge. It installs only bridge files and registration, without touching the
   vault or browser profiles. The manifest permits only Fakkarni's fixed extension ID.
4. The desktop-session diagnostic is saved to
   `artifacts/bridge-diag/desktop-install.json`. Check that both browser registrations,
   the manifest, host executable, and desktop executable are present and valid.
5. Fully exit the browsers, including background processes, then launch normally
   and test the extension. Reboot and test again before considering the recurring
   startup problem resolved.

`scripts/test-browser-bridge.ps1` is a read-only registration/file check unless
`-Probe` is supplied. The optional probe sends only a synthetic origin and reports
protocol status and match count. Neither mode prints passwords or account details.

Regression checks:

- `powershell -NoProfile -ExecutionPolicy Bypass -File tests/native-host-registration.ps1`
  creates a unique synthetic registry branch, checks missing ancestors, both views,
  repeated installation and user-only scope, then removes that test branch.
- `python tests/bridge-launcher.py` uses stub installers in synthetic directories
  to verify spaced paths, success reporting and stopping on failure. It does not
  touch the production registration.

User acceptance remains pending until the normal desktop installation report is
healthy and the extension works after a real Windows restart.
