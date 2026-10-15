# Contributing to Fakkarni

Use synthetic credentials for development and tests. Never commit vaults, key files, plaintext exports, tokens, or personal screenshots. The runtime must remain offline: no telemetry, remote assets, sync, or update requests.

Use one focused commit per completed task, with a short prefix such as `docs:`, `feat:`, `fix:`, or `test:`. Run checks relevant to the change before publishing it. Track incomplete security work explicitly rather than claiming production readiness.

## Development

Install Node.js 24 LTS, stable Rust for the Windows MSVC target, the Visual Studio C++ build tools and a Windows SDK, and WebView2. Follow [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```powershell
npm ci
npm run desktop
```

If npm is available only in the workspace bootstrap tools, use `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/npm.ps1` in place of `npm`. The wrapper adds the standard per-user Cargo location to PATH for the desktop CLI. This process-only flag runs the reviewed project script without changing the machine's execution policy.

`npm run dev` is a nonfunctional browser design preview; its credential inputs are disabled. Use the native desktop application to exercise the vault.

```powershell
npm run build
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The vault core can be tested separately with `cargo test -p fakkarni-vault`. Python and PyKeePass are optional, development-only tools for the independent interoperability check described in `docs/LIBRARY-DECISION.md`.

Build an unpackaged executable with `npm run desktop:build`. Signing and installer distribution are deferred.
