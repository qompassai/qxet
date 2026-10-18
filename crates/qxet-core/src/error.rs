//! Typed errors for qxet-core.
//!
//! Expected failures (bad input, corrupt store data, I/O problems) are
//! values of this enum, never panics. Callers decide recovery; the CLI
//! maps every variant to a non-zero exit code.

use std::path::PathBuf;

/// Everything that can go wrong in a qxet-core operation.
#[derive(Debug, thiserror::Error)]
pub enum QxetError {
    /// The store's `config.toml` disagrees with this build's chunking
    /// parameters; mixing chunkings in one store would corrupt dedup.
    #[error("store config mismatch: {0}")]
    ConfigMismatch(String),

    /// A stored chunk's bytes do not hash to the name they are filed
    /// under, or a chunk file's size disagrees with its manifest entry.
    /// The offending chunk hash is always named.
    #[error("corrupt chunk {hash}: {reason}")]
    CorruptChunk {
        /// SHA-256 (lowercase hex) of the chunk that failed verification.
        hash: String,
        /// What was wrong with it.
        reason: String,
    },

    /// A manifest cannot be parsed or fails structural validation.
    #[error("corrupt manifest {id}: {reason}")]
    CorruptManifest {
        /// Manifest ID (file stem) that failed.
        id: String,
        /// What was wrong with it.
        reason: String,
    },

    /// Chunking parameters outside the algorithm's accepted ranges.
    #[error("invalid chunk parameters: {0}")]
    InvalidChunkParams(String),

    /// A content/manifest ID that is not exactly 64 hex characters.
    #[error("invalid id {0:?}: expected 64 hex characters, optionally prefixed with `qxet:`")]
    InvalidId(String),

    /// An artifact name that is empty, too long, or contains a path
    /// separator or traversal component.
    #[error("invalid artifact name {0:?}")]
    InvalidName(String),

    /// Filesystem failure, with the operation that was attempted.
    #[error("i/o error during {op} on {path}: {source}")]
    Io {
        /// Operation being performed (e.g. "read chunk").
        op: &'static str,
        /// Path involved.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// A chunk file referenced by a manifest does not exist.
    #[error("missing chunk {0}")]
    MissingChunk(String),

    /// No manifest exists for the requested ID.
    #[error("no such artifact: {0}")]
    NoSuchArtifact(String),

    /// No `.qxet/` store found from the given directory upward.
    #[error("no qxet store found from {0} upward (run `qxet init` first)")]
    StoreNotFound(PathBuf),

    /// A named bound was exceeded (chunk count, manifest size/count).
    #[error("limit exceeded: {0}")]
    TooLarge(String),
}

/// Result alias for qxet-core operations.
pub type Result<T> = std::result::Result<T, QxetError>;

impl QxetError {
    /// Build an `Io` error with operation context.
    pub(crate) fn io(op: &'static str, path: &std::path::Path, source: std::io::Error) -> Self {
        QxetError::Io {
            op,
            path: path.to_path_buf(),
            source,
        }
    }
}
