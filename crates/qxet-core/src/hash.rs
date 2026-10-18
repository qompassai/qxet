//! SHA-256 helpers and content-ID validation.
//!
//! Chunk IDs and manifest IDs are the lowercase hex encoding of a
//! SHA-256 digest. IDs are used to build filesystem paths, so an ID is
//! validated as exactly 64 hex characters before it ever touches a
//! path; that check is the store's path-traversal boundary.

use sha2::{Digest, Sha256};

use crate::error::{QxetError, Result};

/// Number of hex characters in a SHA-256 content ID.
pub const ID_HEX_CHARS: usize = 64;

/// SHA-256 of `data`, lowercase hex (64 characters).
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(ID_HEX_CHARS);
    for byte in digest {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0xf) as u32, 16).unwrap_or('0'));
    }
    out
}

/// True iff `s` is exactly 64 ASCII hex characters.
pub fn is_hex_id(s: &str) -> bool {
    s.len() == ID_HEX_CHARS && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Normalize a user-supplied ID: strip an optional `qxet:` prefix,
/// require 64 hex characters, and return the lowercase canonical form.
///
/// # Errors
/// [`QxetError::InvalidId`] for anything else, including strings with
/// path separators or traversal components.
pub fn normalize_id(raw: &str) -> Result<String> {
    let stripped = raw.strip_prefix("qxet:").unwrap_or(raw);
    if !is_hex_id(stripped) {
        return Err(QxetError::InvalidId(raw.to_string()));
    }
    Ok(stripped.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_empty_matches_known_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn normalize_id_accepts_bare_and_prefixed() {
        let id = "AB".repeat(32);
        assert_eq!(normalize_id(&id).expect("hex"), "ab".repeat(32));
        assert_eq!(
            normalize_id(&format!("qxet:{id}")).expect("prefixed"),
            "ab".repeat(32)
        );
    }

    #[test]
    fn normalize_id_rejects_traversal_and_short_ids() {
        for bad in ["../ab", "ab", "", "qxet:", &"a".repeat(63), &"g".repeat(64)] {
            assert!(normalize_id(bad).is_err(), "accepted {bad:?}");
        }
    }
}
