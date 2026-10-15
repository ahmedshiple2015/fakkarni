# Provisional vault library decision

The initial implementation pins Rust `keepass` to **0.15.2** with its `save_kdbx4` feature. It creates KDBX 4.1 databases using Argon2id (three passes, 64 MiB, parallelism one). KDBX keeps the storage format independent of Fakkarni rather than inventing cryptography or a proprietary vault format.

This is a **prototype decision, not a production security endorsement**. Upstream describes its writing support as experimental: [keepass-rs upstream](https://github.com/sseemayer/keepass-rs). Its MIT license is compatible with this project's GPL-3.0-only distribution, subject to preserving notices and reviewing the complete dependency tree.

## Current scope

Only the fixed profile created by Fakkarni is supported in this milestone: KDBX 4.1, AES-256, no compression, Argon2id version 0x13, three passes, 64 MiB, parallelism one, and a 32-byte salt. Before invoking the library parser, a strict outer-header preflight rejects unsupported profiles, duplicate or unknown fields, excessive lengths, and altered KDF resource parameters. This profile does not authenticate a file's creator; cryptographic authentication remains the library's responsibility. Do not overwrite valuable existing KeePass databases: attachment preservation, nested groups, history, custom fields, key files, composite keys, and other clients' optional metadata need separate compatibility tests before general import/edit support.

The application uses an exclusive sidecar lock to refuse a second cooperating Fakkarni writer, encrypted temporary files and atomic replacement, plus a file fingerprint check before saving. A failed save does not commit an in-memory edit. The sidecar lock does not constrain other editors. There remains a race between fingerprint comparison and replacement if another application writes at precisely that point; concurrent editing across different clients is unsupported.

## Validation

`cargo test -p fakkarni-vault` covers synthetic roundtrip create/edit/reopen, wrong password and corruption rejection, lock enforcement, second-writer refusal, refusing to overwrite an existing vault, preservation of an externally changed file, in-memory rollback, plaintext absence from the saved file, and generator length/alphabet boundaries. These tests do not prove secure memory erasure or cryptographic correctness.

For an independent reader check, run:

```text
cargo test -p fakkarni-vault --test vault generate_synthetic_interop_fixture -- --ignored
python scripts/verify-kdbx.py
```

The fixture uses only fixed synthetic credentials, including Arabic and emoji, and is written under ignored `artifacts/interop/`. The Python script accepts no user vault path. It uses [PyKeePass](https://libkeepass.github.io/pykeepass/) to independently verify KDBX version, encryption, numeric Argon2id parameters, and all saved entry fields. PyKeePass is a development-only dependency, never part of the desktop runtime.

On 2026-10-15, the workspace suite passed (11 tests, one ignored fixture generator); the fixture generator was then run explicitly and passed. Independent reading with PyKeePass 4.2.0 passed all version, encryption, KDF, and Unicode field assertions. This verifies this synthetic profile in an independent implementation; GUI interoperability with KeePassXC/KeePass, arbitrary import preservation, and production security review remain outstanding.

## Release gates

Before recommending storage of real credentials:

- Complete independent opening and resaving in KeePassXC and KeePass, including edited Unicode credentials and repeated writes.
- Confirm field protection and KDF parameters through independent inspection.
- Exercise interrupted writes, disk-full and permission failures, backup recovery, and Windows session locking.
- Review the strict preflight and parser resource limits independently. This milestone rejects compressed input and fixes KDF costs; relaxing those restrictions for imports will require explicit CPU, memory, and decompressed-size limits.
- Review secret lifetime and allocations throughout the dependency and IPC boundaries; dropping a database is not proof of comprehensive memory zeroization.
- Review upstream advisories, the pinned dependency tree, licensing notices, and obtain an independent security review.

If the experimental writer cannot meet these gates, replace it or use an established maintained implementation before shipping, while retaining KDBX as the interoperability goal.

