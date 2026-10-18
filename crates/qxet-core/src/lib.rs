//! qxet-core: a local content-addressed store for large binary
//! artifacts (DESIGN.md, v0.1).
//!
//! Files are split with FastCDC content-defined chunking, each chunk
//! is addressed by its SHA-256, identical chunks are stored once, and
//! a JSON manifest per artifact records the ordered chunk list. Reads
//! verify every chunk hash and fail closed on any mismatch. There is
//! no network surface in v0.1; sync is v0.2.
//!
//! Bounds are named constants, not magic numbers: chunk sizes below,
//! manifest limits, and store walk limits in [`store`].

#![forbid(unsafe_code)]

pub mod chunker;
pub mod error;
pub mod gear;
pub mod hash;
pub mod manifest;
pub mod store;

use chunker::ChunkerParams;

/// Minimum chunk size for qxet stores (DESIGN.md).
pub const CHUNK_MIN_BYTES: usize = 16 * 1024;
/// Average chunk size for qxet stores (DESIGN.md).
pub const CHUNK_AVG_BYTES: usize = 64 * 1024;
/// Maximum chunk size for qxet stores (DESIGN.md). Also the largest
/// single in-memory buffer the store ever holds for chunk data.
pub const CHUNK_MAX_BYTES: usize = 256 * 1024;

/// Largest manifest the store will read or write, in bytes. Sized to
/// hold [`MAX_CHUNKS_PER_ARTIFACT`] entries at ~110 bytes each.
pub const MAX_MANIFEST_BYTES: usize = 128 * 1024 * 1024;
/// Most chunks one artifact may reference (at minimum chunk size this
/// still covers 16 GiB; at the average size, 64 GiB).
pub const MAX_CHUNKS_PER_ARTIFACT: usize = 1_048_576;
/// Most manifests one store operation will walk.
pub const MAX_MANIFEST_COUNT: usize = 100_000;
/// Longest artifact name, in bytes (a single filesystem component).
pub const MAX_NAME_BYTES: usize = 255;

/// The chunking parameters every qxet v0.1 store is built with.
/// Stores record their parameters in `config.toml` and refuse to open
/// under a build that disagrees.
pub fn default_chunker_params() -> ChunkerParams {
    ChunkerParams {
        avg_size_bytes: CHUNK_AVG_BYTES,
        max_size_bytes: CHUNK_MAX_BYTES,
        min_size_bytes: CHUNK_MIN_BYTES,
    }
}
