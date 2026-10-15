"""Independently verify ONLY the synthetic Rust integration-test fixture.

Install pykeepass in an isolated environment, generate the fixture with:
cargo test -p fakkarni-vault --test vault generate_synthetic_interop_fixture -- --ignored
Then run this script from the repository root. No user vault path is accepted.
"""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
# Optional local developer install; no Python dependency enters the shipped app.
sys.path.insert(0, str(ROOT / ".tools" / "pykeepass"))
from pykeepass import PyKeePass

path = ROOT / "artifacts" / "interop" / "synthetic.kdbx"
vault = PyKeePass(str(path), password="synthetic test master password")
assert vault.version == (4, 1), vault.version
assert len(vault.entries) == 1
entry = vault.entries[0]
assert entry.title == "Synthetic فكرني 🔐"
assert entry.username == "مستخدم-synthetic@example.test"
assert entry.password == "Synthetic-only-كلمة-289!🔑"
assert entry.url == "https://example.test/login"
assert entry.notes == "Synthetic private note\nملاحظة تجريبية — café"
# PyKeePass 4.2.0's convenience kdf_algorithm property only handles 4.0.
# Inspect its independently parsed variant dictionary for this 4.1 fixture.
params = vault.kdbx.header.value.dynamic_header.kdf_parameters.data.dict
assert params['$UUID'].value == bytes.fromhex('9e298b1956db4773b23dfc3ec6f0a1e6')
assert params['I'].value == 3
assert params['M'].value == 64 * 1024 * 1024
assert params['P'].value == 1
assert params['V'].value == 0x13
assert len(params['S'].value) == 32
assert vault.encryption_algorithm == 'aes256'
print("PASS: independent PyKeePass reader verified synthetic KDBX4.1, Argon2id parameters, and Unicode entry fields")

