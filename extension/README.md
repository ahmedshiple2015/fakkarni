# Fakkarni for Brave

This first browser milestone only displays matching account names and usernames from the unlocked local desktop vault. It does not read forms, save logins, retrieve passwords, fill credentials, or make network requests.

## Build and load

1. From the repository root, install the existing project dependencies and run `node extension/build.mjs`.
2. Open `brave://extensions`, enable Developer mode, and choose **Load unpacked**.
3. Select this `extension` directory, not its `dist` subdirectory.
4. Install the desktop native bridge, then open and unlock Fakkarni.
5. Open an HTTPS website and click the extension. Use **Refresh accounts** after unlocking or changing the vault.

The unpacked extension ID is `kieggakkiiegglikoiojkbfabpmmahep`. `manifest.json` contains a public key to keep that ID stable; no private key is included. Native host registration must allow only `chrome-extension://kieggakkiiegglikoiojkbfabpmmahep/`.

The popup sends only the top-level HTTPS origin to `org.fakkarni.bridge` over browser Native Messaging. It does not send the path, query, fragment, page content, or typed credentials. Account suggestions exist only while the popup is open and are never persisted. A second active-tab check prevents displaying a response for a page that changed during the request.

Run `node --test extension/tests/validators.test.mjs` after building. Opening `popup.html` in a normal browser shows a nonfunctional preview with no account access.

On Windows, scripts/register-brave.ps1 registers the host for the current user in both Brave's native-host location and the Chrome-compatible location used by Chromium's Windows launcher. Both point to the same manifest, allowing only the Fakkarni extension. scripts/unregister-brave.ps1 removes both registrations.
