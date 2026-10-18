//! FastCDC content-defined chunking (Xia et al., 2016; rolling-two-bytes
//! variant, 2020), normalization level 1, seed 0.
//!
//! The scan below is a line-faithful port of the reference algorithm's
//! cut routine: sub-minimum cut-point skipping, a stricter mask below
//! the average size and a looser one above it, and a two-byte rolling
//! Gear hash. Cut points for a given input are therefore identical to
//! the reference implementations; the known-answer tests in
//! `tests/chunker_kat.rs` pin that parity.
//!
//! Contract: [`Chunker::cut`] examines a window of at most
//! `max_size_bytes` and returns the length of the next chunk, in
//! `1..=max_size_bytes` for non-empty input and `0` for empty input.
//! Callers streaming a file must only call it with a full window
//! (`max_size_bytes` bytes) or at end-of-input; see `store::put`.

use crate::error::{QxetError, Result};
use crate::gear::{GEAR, GEAR_LS, MASKS};

/// Smallest minimum chunk size the algorithm accepts.
pub const MIN_SIZE_FLOOR_BYTES: usize = 64;
/// Largest minimum chunk size the algorithm accepts.
pub const MIN_SIZE_CEILING_BYTES: usize = 1_048_576;
/// Smallest average chunk size the algorithm accepts.
pub const AVG_SIZE_FLOOR_BYTES: usize = 256;
/// Largest average chunk size the algorithm accepts.
pub const AVG_SIZE_CEILING_BYTES: usize = 4_194_304;
/// Smallest maximum chunk size the algorithm accepts.
pub const MAX_SIZE_FLOOR_BYTES: usize = 1024;
/// Largest maximum chunk size the algorithm accepts.
pub const MAX_SIZE_CEILING_BYTES: usize = 16_777_216;

/// Chunking parameters, validated by [`Chunker::new`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkerParams {
    /// Target average chunk size in bytes (power-of-two preferred).
    pub avg_size_bytes: usize,
    /// Maximum chunk size in bytes; a hard bound on every chunk.
    pub max_size_bytes: usize,
    /// Minimum chunk size in bytes; cut points are not considered
    /// before this offset within a chunk (sub-minimum skipping).
    pub min_size_bytes: usize,
}

/// A validated FastCDC chunker with precomputed test masks.
#[derive(Clone, Debug)]
pub struct Chunker {
    params: ChunkerParams,
    mask_l: u64,
    mask_l_ls: u64,
    mask_s: u64,
    mask_s_ls: u64,
}

/// Rounded base-2 logarithm, matching the reference implementation's
/// mask-bucket selection (closest bucket, not floor).
fn logarithm2(value: usize) -> u32 {
    (value as f64).log2().round() as u32
}

impl Chunker {
    /// Create a chunker, rejecting parameters outside the algorithm's
    /// accepted ranges or with odd sizes (the two-byte scan requires
    /// even sizes so pair boundaries line up).
    ///
    /// # Errors
    /// [`QxetError::InvalidChunkParams`] naming the offending parameter.
    pub fn new(params: ChunkerParams) -> Result<Self> {
        let checks = [
            (
                "min",
                params.min_size_bytes,
                MIN_SIZE_FLOOR_BYTES,
                MIN_SIZE_CEILING_BYTES,
            ),
            (
                "avg",
                params.avg_size_bytes,
                AVG_SIZE_FLOOR_BYTES,
                AVG_SIZE_CEILING_BYTES,
            ),
            (
                "max",
                params.max_size_bytes,
                MAX_SIZE_FLOOR_BYTES,
                MAX_SIZE_CEILING_BYTES,
            ),
        ];
        for (name, value, floor, ceiling) in checks {
            if !(floor..=ceiling).contains(&value) {
                return Err(QxetError::InvalidChunkParams(format!(
                    "{name} size {value} outside {floor}..={ceiling}"
                )));
            }
            if value % 2 != 0 {
                return Err(QxetError::InvalidChunkParams(format!(
                    "{name} size {value} must be even"
                )));
            }
        }
        if params.min_size_bytes > params.max_size_bytes {
            return Err(QxetError::InvalidChunkParams(format!(
                "min size {} exceeds max size {}",
                params.min_size_bytes, params.max_size_bytes
            )));
        }
        // Normalization level 1: strict mask one bucket above the
        // average, loose mask one bucket below.
        let bits = logarithm2(params.avg_size_bytes);
        let mask_s = MASKS[(bits + 1) as usize];
        let mask_l = MASKS[(bits - 1) as usize];
        Ok(Chunker {
            params,
            mask_l,
            mask_l_ls: mask_l << 1,
            mask_s,
            mask_s_ls: mask_s << 1,
        })
    }

    /// The parameters this chunker was validated with.
    pub fn params(&self) -> ChunkerParams {
        self.params
    }

    /// Length of the next chunk within `window`.
    ///
    /// `window` may be any length; only its first
    /// `max_size_bytes` are examined. Returns `0` iff `window` is
    /// empty. The returned length never exceeds `window.len()`.
    pub fn cut(&self, window: &[u8]) -> usize {
        let min_size = self.params.min_size_bytes;
        let avg_size = self.params.avg_size_bytes;
        let max_size = self.params.max_size_bytes;
        let mut remaining = window.len();
        if remaining <= min_size {
            return remaining;
        }
        let mut center = avg_size;
        if remaining > max_size {
            remaining = max_size;
        } else if remaining < center {
            center = remaining;
        }
        let src = &window[..remaining];
        let limit_strict = center / 2;
        let limit_loose = remaining / 2;
        let mut index = min_size / 2;
        let mut hash: u64 = 0;
        while index < limit_strict {
            let a = index * 2;
            hash = (hash << 2).wrapping_add(GEAR_LS[src[a] as usize]);
            if hash & self.mask_s_ls == 0 {
                return a;
            }
            hash = hash.wrapping_add(GEAR[src[a + 1] as usize]);
            if hash & self.mask_s == 0 {
                return a + 1;
            }
            index += 1;
        }
        while index < limit_loose {
            let a = index * 2;
            hash = (hash << 2).wrapping_add(GEAR_LS[src[a] as usize]);
            if hash & self.mask_l_ls == 0 {
                return a;
            }
            hash = hash.wrapping_add(GEAR[src[a + 1] as usize]);
            if hash & self.mask_l == 0 {
                return a + 1;
            }
            index += 1;
        }
        remaining
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params() -> ChunkerParams {
        ChunkerParams {
            avg_size_bytes: 256,
            max_size_bytes: 1024,
            min_size_bytes: 64,
        }
    }

    #[test]
    fn rejects_odd_and_out_of_range_sizes() {
        let mut p = test_params();
        p.min_size_bytes = 63;
        assert!(Chunker::new(p).is_err());
        p = test_params();
        p.avg_size_bytes = 128;
        assert!(Chunker::new(p).is_err());
        p = test_params();
        p.max_size_bytes = 512;
        assert!(Chunker::new(p).is_err());
        p = test_params();
        p.min_size_bytes = 2048;
        assert!(Chunker::new(p).is_err());
    }

    #[test]
    fn masks_match_reference_for_known_averages() {
        // Reference test_masks: avg 256 -> MASKS[9]/MASKS[7].
        let c = Chunker::new(test_params()).expect("valid params");
        assert_eq!(c.mask_s, MASKS[9]);
        assert_eq!(c.mask_l, MASKS[7]);
        // avg 16384 -> MASKS[15]/MASKS[13].
        let c = Chunker::new(ChunkerParams {
            avg_size_bytes: 16384,
            max_size_bytes: 32768,
            min_size_bytes: 8192,
        })
        .expect("valid params");
        assert_eq!(c.mask_s, MASKS[15]);
        assert_eq!(c.mask_l, MASKS[13]);
        // Non-power-of-two avg 12288 rounds log2 up to 14 (reference
        // regression guard, issue #51): same masks as 16384.
        let c = Chunker::new(ChunkerParams {
            avg_size_bytes: 12288,
            max_size_bytes: 49152,
            min_size_bytes: 3072,
        })
        .expect("valid params");
        assert_eq!(c.mask_s, MASKS[15]);
        assert_eq!(c.mask_l, MASKS[13]);
        // qxet's own parameters: avg 64 KiB -> MASKS[17]/MASKS[15].
        let c = Chunker::new(ChunkerParams {
            avg_size_bytes: 65536,
            max_size_bytes: 262144,
            min_size_bytes: 16384,
        })
        .expect("valid params");
        assert_eq!(c.mask_s, MASKS[17]);
        assert_eq!(c.mask_l, MASKS[15]);
    }

    #[test]
    fn empty_and_tiny_windows_are_whole() {
        let c = Chunker::new(test_params()).expect("valid params");
        assert_eq!(c.cut(&[]), 0);
        assert_eq!(c.cut(&[7u8; 10]), 10);
        assert_eq!(c.cut(&[7u8; 64]), 64);
    }
}
