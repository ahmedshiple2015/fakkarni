//! Strict resource preflight for the small, fixed KDBX profile created by this prototype.
//! Cryptographic header authentication remains the responsibility of keepass-rs.
use crate::{Result, VaultError};
use std::collections::HashSet;

const AES256: &[u8] = &[
    0x31, 0xc1, 0xf2, 0xe6, 0xbf, 0x71, 0x43, 0x50, 0xbe, 0x58, 0x05, 0x21, 0x6a, 0xfc, 0x5a, 0xff,
];
const ARGON2ID: &[u8] = &[
    0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0, 0xa1, 0xe6,
];

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or(VaultError::Unlock)?;
        let value = self.bytes.get(self.offset..end).ok_or(VaultError::Unlock)?;
        self.offset = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn length(&mut self) -> Result<usize> {
        let value: [u8; 4] = self.take(4)?.try_into().map_err(|_| VaultError::Unlock)?;
        Ok(u32::from_le_bytes(value) as usize)
    }
    fn field(&mut self) -> Result<&'a [u8]> {
        let length = self.length()?;
        self.take(length)
    }
}

pub(crate) fn validate(bytes: &[u8]) -> Result<()> {
    let mut reader = Reader { bytes, offset: 0 };
    if reader.take(12)?
        != [
            0x03, 0xd9, 0xa2, 0x9a, 0x67, 0xfb, 0x4b, 0xb5, 0x01, 0x00, 0x04, 0x00,
        ]
    {
        return Err(VaultError::Unlock);
    }
    let mut seen = HashSet::new();
    loop {
        if reader.offset > 4096 {
            return Err(VaultError::Unlock);
        }
        let tag = reader.byte()?;
        let value = reader.field()?;
        if !seen.insert(tag) || value.len() > 2048 {
            return Err(VaultError::Unlock);
        }
        match tag {
            0 if value.is_empty() || value == b"\r\n\r\n" => break,
            2 if value == AES256 => (),
            3 if value == [0, 0, 0, 0] => (),
            4 if value.len() == 32 => (),
            7 if value.len() == 16 => (),
            11 => validate_kdf(value)?,
            _ => return Err(VaultError::Unlock),
        }
    }
    if seen.len() != 6 || ![0, 2, 3, 4, 7, 11].iter().all(|tag| seen.contains(tag)) {
        return Err(VaultError::Unlock);
    }
    Ok(())
}

fn validate_kdf(bytes: &[u8]) -> Result<()> {
    let mut reader = Reader { bytes, offset: 0 };
    if reader.take(2)? != [0, 1] {
        return Err(VaultError::Unlock);
    }
    let mut seen = HashSet::new();
    loop {
        let kind = reader.byte()?;
        if kind == 0 {
            break;
        }
        let key = reader.field()?;
        let value = reader.field()?;
        if !seen.insert(key) {
            return Err(VaultError::Unlock);
        }
        let valid = match (kind, key) {
            (0x42, b"$UUID") => value == ARGON2ID,
            (0x42, b"S") => value.len() == 32,
            (0x04, b"V") => value == 0x13_u32.to_le_bytes(),
            (0x04, b"P") => value == 1_u32.to_le_bytes(),
            (0x05, b"I") => value == 3_u64.to_le_bytes(),
            (0x05, b"M") => value == (64_u64 * 1024 * 1024).to_le_bytes(),
            _ => false,
        };
        if !valid {
            return Err(VaultError::Unlock);
        }
    }
    if seen.len() != 6 || reader.offset != bytes.len() {
        return Err(VaultError::Unlock);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_and_oversized_headers_are_rejected_without_panicking() {
        for length in 0..64 {
            assert!(validate(&vec![0; length]).is_err());
        }
        let mut input = vec![
            0x03, 0xd9, 0xa2, 0x9a, 0x67, 0xfb, 0x4b, 0xb5, 0x01, 0x00, 0x04, 0x00, 11,
        ];
        input.extend(u32::MAX.to_le_bytes());
        assert!(validate(&input).is_err());
    }
    #[test]
    fn rejects_unsupported_or_excessive_kdf_before_derivation() {
        fn pair(kind: u8, key: &[u8], value: &[u8]) -> Vec<u8> {
            let mut bytes = vec![kind];
            bytes.extend((key.len() as u32).to_le_bytes());
            bytes.extend(key);
            bytes.extend((value.len() as u32).to_le_bytes());
            bytes.extend(value);
            bytes
        }
        let mut bytes = vec![0, 1];
        bytes.extend(pair(0x42, b"$UUID", ARGON2ID));
        bytes.extend(pair(0x42, b"S", &[1; 32]));
        bytes.extend(pair(0x04, b"V", &0x13_u32.to_le_bytes()));
        bytes.extend(pair(0x04, b"P", &1_u32.to_le_bytes()));
        bytes.extend(pair(0x05, b"I", &3_u64.to_le_bytes()));
        bytes.extend(pair(0x05, b"M", &(64_u64 * 1024 * 1024).to_le_bytes()));
        bytes.push(0);
        assert!(validate_kdf(&bytes).is_ok());
        let end = bytes.len() - 9;
        bytes[end..end + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(validate_kdf(&bytes).is_err());
    }
}
