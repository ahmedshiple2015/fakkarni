# Fakkarni for Brave and Edge

The extension displays matching accounts and fills a login when you choose it. It makes no network requests and does not save logins automatically. A desktop preference can expose account names, usernames, and website addresses while the vault is locked; this metadata is stored separately on the PC and is off by default. Passwords always require an unlocked vault. Choosing an account fills directly when unlocked; otherwise Fakkarni opens for unlocking and then continues that fill automatically.

## Build and load

1. From the repository root, install the existing project dependencies and run `node extension/build.mjs`.
2. Open `brave://extensions`, enable Developer mode, and choose **Load unpacked**.
3. Select this `extension` directory, not its `dist` subdirectory.
4. Install the desktop native bridge, then open and unlock Fakkarni.
5. Open an HTTPS website and click the extension. Use **Refresh accounts** after unlocking or changing the vault.
6. Choose an account to fill. If the vault is locked, Fakkarni opens in front so you can unlock it. The extension then fills the original page and returns to the browser. Review the values and submit the form yourself. No separate approval button is required.

The unpacked extension ID is `kieggakkiiegglikoiojkbfabpmmahep`. `manifest.json` contains a public key to keep that ID stable; no private key is included. Native host registration must allow only `chrome-extension://kieggakkiiegglikoiojkbfabpmmahep/`.

The popup sends only the top-level HTTPS origin to `org.fakkarni.bridge` over browser Native Messaging. It does not send the path, query, fragment, page content, or typed credentials. Account suggestions exist only while the popup is open and are never persisted by the extension. A second active-tab check prevents displaying a response for a page that changed during the request.

Clicking an account starts a background handoff that survives the popup closing when the desktop app gains focus. The worker acknowledges queueing before beginning the native handoff, so focusing the desktop cannot interrupt the popup response. The request expires after two minutes and is cancelled on navigation, reload, closing the tab, or another fill request. The background worker accepts a password only from the selected account's one-use poll response and never sends it to the popup, logs it, or stores it. JavaScript memory cannot guarantee immediate erasure; credentials are released after the injection attempt.

Filling uses the `activeTab` permission granted by opening the extension, plus `scripting`. There are no broad website permissions or always-running content scripts. The main-frame document ID, full URL, and exact HTTPS origin must still match the page selected by the user. The script fills only one visible, enabled, writable password field with one unambiguous username field in the same form. Signup/password-change forms, ambiguous forms, password-only steps, iframes, and forms without a form element are refused. It never submits, clicks a website control, or follows navigation. A green check badge means filled; an exclamation badge means reopen the extension or fill an unsupported form manually.

Browsers do not generally allow a popup to open itself on arbitrary sites. Open Fakkarni's toolbar popup and choose an account to initiate the handoff. The desktop application may appear automatically as part of this explicit click.

Run `node --test extension/tests/*.test.mjs` after building. Tests use synthetic forms and mocked browser APIs. Opening `popup.html` in a normal browser shows a nonfunctional preview with no account access. Reload the extension after updating, accepting the new scripting permission if requested.

On Windows, scripts/register-brave.ps1 registers the host for the current user in both Brave's native-host location and the Chrome-compatible location used by Chromium's Windows launcher. Both point to the same manifest, allowing only the Fakkarni extension. scripts/unregister-brave.ps1 removes both registrations.

## Microsoft Edge

The same unpacked extension also works in Edge. Run scripts/register-edge.ps1 to install its current-user native-host registration, then load the extension folder at edge://extensions with Developer mode enabled. Keep the desktop vault unlocked and click Refresh accounts. Remove the Edge registration with scripts/unregister-edge.ps1. The native manifest and host permit the same fixed extension ID; no additional origins or permissions are added.

Revealed password inputs (`type="text"`) are supported when they retain an explicit current-password autocomplete hint or a recognized password name/ID. They are excluded from username selection, and their visible state is preserved. Arbitrary text inputs and ambiguous/password-confirmation forms remain unsupported.
