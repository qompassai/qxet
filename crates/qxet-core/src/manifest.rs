//! The artifact manifest: the JSON document that describes one stored
//! file as an ordered list of chunks.
//!
//! Manifests are serialized canonically — `serde` emits struct fields
//! in declaration order, and the fields below are declared
//! alphabetically — so the same logical manifest always produces the
//! same bytes. The manifest's content ID is the SHA-256 of those
//! canonical bytes, and the manifest is stored under that ID.
//!
//! A parsed manifest is untrusted input (anyone can edit files inside
//! a store directory), so [`Manifest::validate`] re-checks every
//! structural invariant before any chunk is read: names cannot escape
//! the output directory, offsets must tile the file exactly, and chunk
//! lengths must be non-zero and within the chunker's maximum.

use serde::{Deserialize, Serialize};

use crate::error::{QxetError, Result};
use crate::hash::{is_hex_id, sha256_hex};
use crate::{MAX_CHUNKS_PER_ARTIFACT, MAX_MANIFEST_BYTES, MAX_NAME_BYTES};

/// One chunk of an artifact, in file order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChunkRef {
    /// SHA-256 of the chunk's bytes, lowercase hex; also its store name.
    pub hash: String,
    /// Length of the chunk in bytes (1..=CHUNK_MAX_BYTES).
    pub length: u64,
    /// Offset of the chunk within the artifact in bytes.
    pub offset: u64,
}

/// A stored artifact's manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Manifest {
    /// Ordered chunks that reassemble into the artifact.
    pub chunks: Vec<ChunkRef>,
    /// When the artifact was stored, seconds since the Unix epoch.
    pub created_unix_secs: u64,
    /// Original file name (a bare name, never a path).
    pub name: String,
    /// Total artifact size in bytes; equals the sum of chunk lengths.
    pub size_bytes: u64,
}

/// Validate an artifact name: a bare file name, never a path.
///
/// Rejects empty names, names over [`MAX_NAME_BYTES`], `.`/`..`, and
/// any name containing a path separator — such a name could make
/// `qxet get` write outside the caller's chosen directory.
///
/// # Errors
/// [`QxetError::InvalidName`].
pub fn validate_artifact_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0');
    if bad {
        return Err(QxetError::InvalidName(name.to_string()));
    }
    Ok(())
}

impl Manifest {
    /// Canonical JSON bytes of this manifest.
    ///
    /// Serialization of this struct cannot fail in practice (plain data,
    /// no custom serializers); the error is still propagated rather
    /// than unwrapped.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| QxetError::CorruptManifest {
            id: String::new(),
            reason: format!("serialization failed: {e}"),
        })
    }

    /// This manifest's content ID: SHA-256 of its canonical bytes.
    pub fn content_id(&self) -> Result<String> {
        Ok(sha256_hex(&self.to_canonical_bytes()?))
    }

    /// Parse a manifest from bytes and validate its structure.
    ///
    /// # Errors
    /// [`QxetError::TooLarge`] if `bytes` exceeds
    /// [`MAX_MANIFEST_BYTES`]; [`QxetError::CorruptManifest`] on any
    /// parse or validation failure.
    pub fn parse(id_for_errors: &str, bytes: &[u8]) -> Result<Manifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(QxetError::TooLarge(format!(
                "manifest {id_for_errors} is {} bytes, limit is {MAX_MANIFEST_BYTES}",
                bytes.len()
            )));
        }
        let manifest: Manifest =
            serde_json::from_slice(bytes).map_err(|e| QxetError::CorruptManifest {
                id: id_for_errors.to_string(),
                reason: format!("invalid JSON: {e}"),
            })?;
        manifest.validate(id_for_errors)?;
        Ok(manifest)
    }

    /// Check every structural invariant of this manifest.
    ///
    /// # Errors
    /// [`QxetError::CorruptManifest`] naming the first violated rule.
    pub fn validate(&self, id_for_errors: &str) -> Result<()> {
        let corrupt = |reason: String| QxetError::CorruptManifest {
            id: id_for_errors.to_string(),
            reason,
        };
        validate_artifact_name(&self.name)
            .map_err(|_| corrupt(format!("invalid artifact name {:?}", self.name)))?;
        if self.chunks.len() > MAX_CHUNKS_PER_ARTIFACT {
            return Err(corrupt(format!(
                "chunk count {} exceeds limit {MAX_CHUNKS_PER_ARTIFACT}",
                self.chunks.len()
            )));
        }
        let mut expected_offset: u64 = 0;
        for chunk in &self.chunks {
            if !is_hex_id(&chunk.hash) || chunk.hash != chunk.hash.to_ascii_lowercase() {
                return Err(corrupt(format!(
                    "chunk hash {:?} is not lowercase hex",
                    chunk.hash
                )));
            }
            if chunk.length == 0 || chunk.length > crate::CHUNK_MAX_BYTES as u64 {
                return Err(corrupt(format!(
                    "chunk {} has length {}, outside 1..={}",
                    chunk.hash,
                    chunk.length,
                    crate::CHUNK_MAX_BYTES
                )));
            }
            if chunk.offset != expected_offset {
                return Err(corrupt(format!(
                    "chunk {} has offset {}, expected {expected_offset}",
                    chunk.hash, chunk.offset
                )));
            }
            expected_offset = expected_offset
                .checked_add(chunk.length)
                .ok_or_else(|| corrupt("chunk offsets overflow u64".to_string()))?;
        }
        if expected_offset != self.size_bytes {
            return Err(corrupt(format!(
                "chunk lengths sum to {expected_offset}, manifest size is {}",
                self.size_bytes
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            chunks: vec![
                ChunkRef {
                    hash: "a".repeat(64),
                    length: 10,
                    offset: 0,
                },
                ChunkRef {
                    hash: "b".repeat(64),
                    length: 5,
                    offset: 10,
                },
            ],
            created_unix_secs: 1_700_000_000,
            name: "model.bin".to_string(),
            size_bytes: 15,
        }
    }

    #[test]
    fn canonical_bytes_have_alphabetical_keys() {
        let bytes = sample().to_canonical_bytes().expect("serializes");
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.starts_with("{\"chunks\":[{\"hash\":"));
        let order: Vec<usize> = [
            "\"chunks\"",
            "\"created_unix_secs\"",
            "\"name\"",
            "\"size_bytes\"",
        ]
        .iter()
        .map(|k| text.find(k).expect("key present"))
        .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "keys out of order: {text}"
        );
    }

    #[test]
    fn round_trip_parse_validates() {
        let m = sample();
        let bytes = m.to_canonical_bytes().expect("serializes");
        let parsed = Manifest::parse("test", &bytes).expect("valid manifest");
        assert_eq!(parsed, m);
    }

    #[test]
    fn rejects_gap_in_offsets_and_zero_length_chunk() {
        let mut m = sample();
        m.chunks[1].offset = 11;
        assert!(m.validate("t").is_err());
        let mut m = sample();
        m.chunks[0].length = 0;
        m.chunks[1].offset = 0;
        m.size_bytes = 5;
        assert!(m.validate("t").is_err());
    }

    #[test]
    fn rejects_traversal_names() {
        for bad in ["", ".", "..", "../evil", "a/b", "a\\b", &"x".repeat(256)] {
            assert!(validate_artifact_name(bad).is_err(), "accepted {bad:?}");
        }
        assert!(validate_artifact_name("weights-ü.bin").is_ok());
    }
}
