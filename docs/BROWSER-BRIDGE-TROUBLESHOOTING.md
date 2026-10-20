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

Open Fakkarni normally and choose **Settings → Browser connection → Repair browser
bridge**. The same action is available on the locked screen, so repair does not
require a vault unlock. It installs and verifies the bridge files and current-user
registrations for Brave and Edge directly from the desktop app, without a terminal,
administrator prompt, or access to vault contents. An existing, unchanged bridge
executable is preserved. The app never closes your browsers automatically; fully
exit and reopen them if they still cannot connect after repair.

The desktop executable's folder must also contain `fakkarni-native-host.exe`.
If it is missing, restore it from the application build/install before repairing.
The button reports that failure rather than registering a missing executable.

The File Explorer repair launcher remains available as a development fallback:

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

## Settings button verification (2026-10-20)

The native repair path passed seven focused checks: manifest identity, a fresh
UUID-scoped registry branch with missing ancestors and incorrect value type,
synthetic file/registration installation, unchanged repeated repair, missing
sidecar/failed registration, metadata replacement failures, and an in-use bridge
executable. No production browser registration or vault was altered by those tests.
All 40 desktop Rust tests, strict workspace Clippy, seven form tests, the frontend
build and synthetic UI checks passed. UI checks covered both entry points,
duplicate clicks, failure/retry, and reporting a completed repair across locking.

Manual acceptance of the Settings action is still pending: open Fakkarni normally,
choose Repair browser bridge, confirm its success message, and check the Brave and
Edge extensions. Repeating repair should succeed without changing vault contents
or terminating browsers. The locked-screen action should work without a vault unlock.

## First cold-start click opens the app but never asks for PIN (2026-10-20)

A temporary stage-only trace captured the first request arriving and creating its
pending fill, followed by `pin_owner_invalid`. The request was cancelled before
calling the PIN provider: capturing the foreground browser after desktop startup
lost the original window. Increasing the startup wait did not address this case.

The native host now captures the foreground browser before launching the desktop
and passes a one-use startup hint bound to that exact fill nonce. The hint contains
only HWND, process ID, Windows ticks and the request nonce; no account, URL or secret.
It cannot initiate an unlock by itself. The desktop checks the pending request,
window/process identity and age again. Restoring foreground is limited to the
original browser, this desktop's startup window, or a transition with no foreground
window; Windows retains final control over foreground access.
Warm requests continue using their current foreground browser. The extension wire
format remains strict and does not accept a supplied window-owner field.

Tests cover hint parsing, exact-request binding, expiry and tick wrapping, mouse
movement versus another foreground app, supported process names, forwarding the original selection after launch,
and cancellation/stale-result handling. Real PIN acceptance remains pending: quit
the app, click Unlock & fill once, verify the native prompt and automatic fill,
then repeat from fully closed state separately in Brave and Edge. Also cancel the
prompt and verify that no password is filled and no vault window appears.

References: [Windows foreground permission](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow)
and [Edge native messaging](https://learn.microsoft.com/en-us/microsoft-edge/extensions/developer-guide/native-messaging).

The normal-desktop report initially showed `hostMatchesBuild: false`, explaining
why the first retest still had no startup hint. Running the repair launcher from
File Explorer changed this to `true`; the next trace contained the native handoff.
A first successful PIN test was followed by intermittent failures in both browsers.
The more precise trace identified `pin_owner_input_changed`: global input ticks
include mouse movement, which incorrectly cancelled startup. The desktop now checks
the actual foreground owner instead and cancels if another app or browser window
has focus. The user confirmed two consecutive cold starts in Brave, including
first-click PIN and automatic filling, but later reported more intermittent Brave
failures. Edge cold starts were reported working. Temporary stage tracing remains
enabled in this debug build during acceptance. A successful build alone is not
proof that the browser uses that build: verify the installed executable in the
normal desktop session.

## Close the extension popup before native PIN handoff

The account popup previously remained open after acknowledging a fill, overlapping
native startup and PIN UI. The extension now uses a named runtime port: the worker
validates and queues the exact tab/document, acknowledges the selection, and the
popup closes automatically. Native BeginFill waits for the accepted popup's actual
disconnect, rather than a timer. A view closed before acceptance cannot trigger a
delayed prompt. Navigation, duplicate messages, malformed acknowledgements and
worker disconnection remain guarded; PIN cancellation never retries a prompt.

All 30 extension tests and the synthetic rendered popup check passed. Checks cover
the close barrier, original-document binding, early disconnect, rejected handoffs,
repeat fills, locked/unlocked labels, no external requests and no overflow. Reload
the unpacked extension to use the new worker; no desktop rebuild or bridge repair
is needed. Repeated Brave acceptance is pending, followed by an Edge regression
check. Temporary tracing will be removed after the full path is accepted.

Port lifetime behavior follows [Chrome's messaging documentation](https://developer.chrome.com/docs/extensions/develop/concepts/messaging#port-lifetime).

## PIN provider ownership before key opening

The popup handoff improved Brave to four successful cold starts out of five in a
user test, but did not meet the acceptance gate. The failed run reached the PIN
provider and returned cancellation. No automatic retry was added: an actual user
cancellation must continue to stop the request.

Interactive key access now sets the HWND and context on the Passport provider
before NCryptOpenKey; the returned key still receives its own UI properties before
decryption. Previously only the key handle had an owner, leaving any key-opening UI
without one. Provider setup failure stops access. This follows the documented
[provider UI context during key opening](https://learn.microsoft.com/en-us/windows/win32/seccng/key-storage-property-identifiers).

An opt-in, thread-scoped debug observer now distinguishes provider opening, key
opening, property setup and private decryption errors using static stages and
numeric status codes only. It restores its previous observer on return or unwind
and does nothing in release builds. No keys, payloads or PINs enter the observer.

Nineteen targeted tests passed, including provider-level owner setup without
opening a stored key, ephemeral software RSA interoperability, observer cleanup,
enrollment preservation and browser cancellation. Strict Clippy and the desktop
build passed. Repeated Brave acceptance and an Edge regression check remain pending.
