# Desktop and extension design

Fakkarni uses a quiet dark interface with its existing purple accent. The desktop and extension share locally bundled tokens in `extension/theme.css`. The desktop imports these through Vite; the extension loads them directly from its own package. System fonts and fixed inline SVG icons require no font, favicon, image, or analytics requests.

The vault view contains search, saved logins, and account details. Settings has a separate navigation button for Windows Hello, tray/startup preferences, browser privacy, and encrypted backup/transfer. Leaving account details hides a revealed password and prevents a pending reveal response from putting it back into the hidden panel. Locking clears the interface and returns to the unlock screen. Settings navigation is unavailable while locked.

The extension makes account selection the main action, with Refresh as a secondary control. Its connection indicator distinguishes checking, unlocked, locked, unsupported page, and unavailable connection. Autofill still requires an account selection, validates the original browser page, and never submits a form.

## Sources and installed skills

The following were inspected at the user's request. Skills were installed into the user's Codex skills directory, not added as dependencies or shipped with Fakkarni. Newly installed skills become available to normal skill discovery on the next turn; applicable instructions were read directly for this change.

| Repository | Installation and use |
| --- | --- |
| [Superdesign](https://github.com/superdesigndev/superdesign) | Original repository is an IDE extension. Installed `superdesign` from its linked [skill repository](https://github.com/superdesigndev/superdesign-skill). Its remote design workflow was not used. |
| [Awesome Codex Skills](https://github.com/composio-community/awesome-codex-skills) | Installed the relevant `brand-guidelines`, `canvas-design`, `theme-factory`, and `webapp-testing` skills. Used the local browser-testing workflow. The collection's unrelated service automations were not installed. |
| [Better Design](https://github.com/marvkr/better-design) | Installed `better-design` and `animate-text`. The skill requires its MCP connection, which is not configured here; its MCP workflow was not run. |
| [Design Taste](https://github.com/madebymustafa/design-taste) | Installed all 24 skills. Applied the core, design-system, accessibility, and motion guidance appropriate to a password manager. |
| [Canvas UI](https://github.com/DavidHDev/canvas-ui) | Component library; no `SKILL.md` found. Reference only. |
| [UI Layouts](https://github.com/ui-layouts/uilayouts) | Component library; no `SKILL.md` found. Reference only. |
| [Motion Primitives](https://github.com/ibelick/motion-primitives) | Component library; no `SKILL.md` found. Reference only. |
| [Paper Shaders](https://github.com/paper-design/shaders) | Shader library; no `SKILL.md` found. Reference only. |
| [Cursify](https://github.com/ui-layouts/cursify) | Cursor animation library; no `SKILL.md` found. Reference only. |

31 skills were installed in total. The runtime remains vanilla TypeScript/CSS with Tauri. No new runtime dependencies, external design-service calls, GPU effects, or custom cursors were added.

## Verification

- Desktop TypeScript/Vite production build and Tauri debug executable build.
- Existing seven desktop editor tests and 18 extension protocol/autofill tests.
- Disposable Playwright browser with synthetic IPC and extension APIs: vault/settings navigation, first-account selection, search, reveal hiding including an outstanding request, editor password preservation, delete cancellation/focus, locked navigation, Hello button availability, narrow layouts, extension connection/empty states, and fill handoff.
- Captured and inspected desktop vault, settings, editor, locked, and minimum-width layouts plus extension unlocked, locked, and unavailable states. These use fabricated accounts only.
- Checked focus indicators, contrast of shared text colors, and reduced-motion handling. Synthetic desktop rendering made no external requests. This is not a full assistive-technology audit or a real Windows Hello authentication test.

After updating, restart the desktop executable and reload the unpacked extension at `brave://extensions` and `edge://extensions`. UI changes do not require rewriting the native bridge registrations.
