# فكرني · Fakkarni

A planned open-source, local-only password manager for Windows, with Brave browser integration.

**Status:** planning foundation. No working vault or browser extension exists yet. Do not store real credentials in development prototypes.

## Agreed first release

- Windows 10/11 desktop application and Brave extension.
- Master-password unlock; automatic lock when Windows locks.
- Encrypted local vault, preferably KDBX after library evaluation.
- Create, edit, search, and generate passwords.
- Save/update prompts for browser logins and explicit click-to-fill.
- Password import and encrypted local backup/restore.
- Clean dark interface with Arabic and English naming.
- GPL-3.0 license selected; include the standard license text before distribution.

No cloud service, telemetry, remote assets, or automatic network update checks. Filling credentials intentionally gives them to the selected website or application during login.

## Later milestones

Windows Hello unlock, Firefox support, foreground-app suggestions, and individually tested filling for desktop applications such as Discord, Steam, and Riot clients.

See [the implementation plan](docs/PLAN.md) and [security design](docs/SECURITY-DESIGN.md).
