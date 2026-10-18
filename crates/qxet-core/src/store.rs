//! The content-addressed store: chunk files, manifest files, and the
//! operations over them (`put`, `get`, `list`, `verify`, `gc`).
//!
//! Layout under the store root (`.qxet/` by default):
//!
//! ```text
//! .qxet/
//!   chunks/ab/cdef...   # chunk bytes, named by SHA-256, 2-hex fan-out
//!   manifests/<id>.json # canonical JSON manifests, named by content ID
//!   config.toml         # chunking parameters the store was built with
//! ```
//!
//! Safety invariants:
//! - Chunk and manifest file names are always validated content IDs
//!   (64 hex chars) before they become paths — see [`crate::hash`].
//! - `get` rehashes every chunk it reads and fails closed on any
//!   mismatch; output is written to a temp file and renamed into place
//!   only after the whole artifact verifies.
//! - `gc` refuses to run if any manifest fails to parse: a partial
//!   mark set could delete live chunks.
//! - All file writes are temp-file + rename within the same directory.

use std::collections::HashSet;
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::chunker::{Chunker, ChunkerParams};
use crate::error::{QxetError, Result};
use crate::hash::{is_hex_id, normalize_id, sha256_hex};
use crate::manifest::{validate_artifact_name, ChunkRef, Manifest};
use crate::{CHUNK_MAX_BYTES, MAX_CHUNKS_PER_ARTIFACT, MAX_MANIFEST_BYTES, MAX_MANIFEST_COUNT};

/// Store directory name created by `qxet init`.
pub const STORE_DIR_NAME: &str = ".qxet";
/// Deepest ancestor search performed by [`Store::discover`].
const DISCOVER_ANCESTOR_LIMIT: usize = 32;
/// Config file format version written by this build.
const CONFIG_VERSION: u32 = 1;

/// Unique suffixes for temp files, unique within this process; the PID
/// in the name separates processes.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_sibling(dir: &Path, tag: &str) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".tmp-{tag}-{}-{n}", std::process::id()))
}

/// On-disk store configuration (`config.toml`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StoreConfig {
    avg_size_bytes: usize,
    max_size_bytes: usize,
    min_size_bytes: usize,
    version: u32,
}

impl StoreConfig {
    fn for_params(params: ChunkerParams) -> Self {
        StoreConfig {
            avg_size_bytes: params.avg_size_bytes,
            max_size_bytes: params.max_size_bytes,
            min_size_bytes: params.min_size_bytes,
            version: CONFIG_VERSION,
        }
    }

    fn params(&self) -> ChunkerParams {
        ChunkerParams {
            avg_size_bytes: self.avg_size_bytes,
            max_size_bytes: self.max_size_bytes,
            min_size_bytes: self.min_size_bytes,
        }
    }
}

/// Outcome of a successful [`Store::put_file`].
#[derive(Clone, Debug)]
pub struct PutOutcome {
    /// True when an identical manifest already existed and nothing
    /// new was written (duplicate puts are idempotent).
    pub already_present: bool,
    /// Number of chunks in the artifact.
    pub chunk_count: usize,
    /// Content ID of the artifact's manifest (bare hex).
    pub id: String,
    /// Artifact name recorded in the manifest.
    pub name: String,
    /// Chunks whose bytes were not already in the store.
    pub new_chunk_count: usize,
    /// Artifact size in bytes.
    pub size_bytes: u64,
}

/// One artifact as reported by [`Store::list`].
#[derive(Clone, Debug)]
pub struct ArtifactInfo {
    /// Chunks referenced by the manifest.
    pub chunk_count: usize,
    /// When the artifact was stored, seconds since the Unix epoch.
    pub created_unix_secs: u64,
    /// Content ID (bare hex).
    pub id: String,
    /// Original file name.
    pub name: String,
    /// Artifact size in bytes.
    pub size_bytes: u64,
    /// Bytes occupied by this artifact's distinct chunks.
    pub stored_bytes: u64,
}

/// Outcome of [`Store::verify`]. Verification reports; it does not
/// stop at the first problem.
#[derive(Clone, Debug, Default)]
pub struct VerifyReport {
    /// Manifests that parsed and were fully checked.
    pub artifacts_checked: usize,
    /// Chunk references whose bytes were rehashed.
    pub chunks_checked: usize,
    /// Hashes of chunks that are missing or fail rehashing.
    pub corrupt_chunks: Vec<String>,
    /// Manifest IDs that are unparseable or stored under a wrong ID.
    pub corrupt_manifests: Vec<String>,
}

impl VerifyReport {
    /// True when nothing corrupt was found.
    pub fn is_clean(&self) -> bool {
        self.corrupt_chunks.is_empty() && self.corrupt_manifests.is_empty()
    }
}

/// Outcome of [`Store::gc`].
#[derive(Clone, Debug, Default)]
pub struct GcReport {
    /// True when nothing was deleted (report-only mode).
    pub dry_run: bool,
    /// Unreferenced chunks found.
    pub orphan_chunk_count: usize,
    /// Total bytes of unreferenced chunks.
    pub orphan_bytes: u64,
    /// Chunks actually deleted (zero on a dry run).
    pub removed_chunk_count: usize,
    /// Bytes actually reclaimed (zero on a dry run).
    pub removed_bytes: u64,
    /// Files under `chunks/` that are not chunk candidates (temp
    /// files, stray files) and were left alone.
    pub skipped_file_count: usize,
}

/// An open qxet store.
#[derive(Debug)]
pub struct Store {
    chunker: Chunker,
    root: PathBuf,
}

impl Store {
    /// Create a store rooted at `dir`/`.qxet`, or open the existing
    /// one there. Idempotent: re-initializing a healthy store succeeds
    /// and changes nothing.
    ///
    /// # Errors
    /// I/O failures, or [`QxetError::ConfigMismatch`] if an existing
    /// store was built with different chunking parameters.
    pub fn init(dir: &Path) -> Result<Store> {
        let root = dir.join(STORE_DIR_NAME);
        fs::create_dir_all(root.join("chunks"))
            .map_err(|e| QxetError::io("create chunks dir", &root, e))?;
        fs::create_dir_all(root.join("manifests"))
            .map_err(|e| QxetError::io("create manifests dir", &root, e))?;
        let config_path = root.join("config.toml");
        let expected = StoreConfig::for_params(crate::default_chunker_params());
        if config_path.is_file() {
            let existing = Self::read_config(&config_path)?;
            if existing != expected {
                return Err(QxetError::ConfigMismatch(format!(
                    "store config {existing:?} != this build's {expected:?}"
                )));
            }
        } else {
            let text = toml::to_string(&expected)
                .map_err(|e| QxetError::ConfigMismatch(format!("cannot serialize config: {e}")))?;
            let tmp = temp_sibling(&root, "config");
            fs::write(&tmp, text).map_err(|e| QxetError::io("write config", &tmp, e))?;
            fs::rename(&tmp, &config_path)
                .map_err(|e| QxetError::io("rename config", &config_path, e))?;
        }
        Self::open(&root)
    }

    /// Open the store whose root is `root` (the `.qxet` directory
    /// itself), validating its config against this build.
    ///
    /// # Errors
    /// Missing directories, unparseable config, or
    /// [`QxetError::ConfigMismatch`].
    pub fn open(root: &Path) -> Result<Store> {
        for sub in ["chunks", "manifests"] {
            let p = root.join(sub);
            if !p.is_dir() {
                return Err(QxetError::io(
                    "open store",
                    &p,
                    std::io::Error::new(std::io::ErrorKind::NotFound, "store subdirectory missing"),
                ));
            }
        }
        let config = Self::read_config(&root.join("config.toml"))?;
        let expected = StoreConfig::for_params(crate::default_chunker_params());
        if config != expected {
            return Err(QxetError::ConfigMismatch(format!(
                "store config {config:?} != this build's {expected:?}"
            )));
        }
        let chunker = Chunker::new(config.params())?;
        Ok(Store {
            chunker,
            root: root.to_path_buf(),
        })
    }

    /// Find a store by walking from `start` up through its ancestors
    /// (at most [`DISCOVER_ANCESTOR_LIMIT`]) looking for `.qxet/`.
    ///
    /// # Errors
    /// [`QxetError::StoreNotFound`] when no ancestor holds a store.
    pub fn discover(start: &Path) -> Result<Store> {
        // A relative start (e.g. ".") has no real parent chain, so
        // absolutize before walking up.
        let absolute = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
        let mut dir = Some(absolute.as_path());
        for _ in 0..DISCOVER_ANCESTOR_LIMIT {
            match dir {
                Some(d) => {
                    let candidate = d.join(STORE_DIR_NAME);
                    if candidate.join("config.toml").is_file() {
                        return Self::open(&candidate);
                    }
                    dir = d.parent();
                }
                None => break,
            }
        }
        Err(QxetError::StoreNotFound(start.to_path_buf()))
    }

    /// The store root (the `.qxet` directory).
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn read_config(path: &Path) -> Result<StoreConfig> {
        let text = fs::read_to_string(path).map_err(|e| QxetError::io("read config", path, e))?;
        toml::from_str(&text).map_err(|e| {
            QxetError::ConfigMismatch(format!("unparseable config {}: {e}", path.display()))
        })
    }

    fn chunk_path(&self, hash: &str) -> PathBuf {
        debug_assert!(is_hex_id(hash));
        self.root.join("chunks").join(&hash[..2]).join(hash)
    }

    fn manifest_path(&self, id: &str) -> PathBuf {
        debug_assert!(is_hex_id(id));
        self.root.join("manifests").join(format!("{id}.json"))
    }

    /// Sorted IDs of all manifests in the store.
    fn manifest_ids(&self) -> Result<Vec<String>> {
        let dir = self.root.join("manifests");
        let mut ids = Vec::new();
        let entries = fs::read_dir(&dir).map_err(|e| QxetError::io("list manifests", &dir, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| QxetError::io("list manifests", &dir, e))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some(stem) = name.strip_suffix(".json") {
                if is_hex_id(stem) {
                    ids.push(stem.to_ascii_lowercase());
                }
            }
        }
        if ids.len() > MAX_MANIFEST_COUNT {
            return Err(QxetError::TooLarge(format!(
                "manifest count {} exceeds limit {MAX_MANIFEST_COUNT}",
                ids.len()
            )));
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Read and fully validate one manifest, including that it is
    /// stored under its own content ID.
    fn read_manifest(&self, id: &str) -> Result<Manifest> {
        let path = self.manifest_path(id);
        let file = fs::File::open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                QxetError::NoSuchArtifact(id.to_string())
            } else {
                QxetError::io("read manifest", &path, e)
            }
        })?;
        let mut bytes = Vec::new();
        file.take((MAX_MANIFEST_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| QxetError::io("read manifest", &path, e))?;
        let manifest = Manifest::parse(id, &bytes)?;
        let actual = manifest.content_id()?;
        if actual != id {
            return Err(QxetError::CorruptManifest {
                id: id.to_string(),
                reason: format!("stored under {id} but hashes to {actual}"),
            });
        }
        Ok(manifest)
    }

    /// Store one chunk's bytes if not already present. Returns true
    /// when the chunk was newly written. An existing chunk file with
    /// the wrong size is corrupt by definition (its name is its hash)
    /// and is replaced; an existing file with the right size is
    /// trusted until `get`/`verify` rehash it.
    fn write_chunk(&self, hash: &str, bytes: &[u8]) -> Result<bool> {
        let path = self.chunk_path(hash);
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() == bytes.len() as u64 {
                return Ok(false);
            }
        }
        let fanout = path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| QxetError::Io {
                op: "resolve chunk dir",
                path: path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "chunk path has no parent",
                ),
            })?;
        fs::create_dir_all(&fanout).map_err(|e| QxetError::io("create chunk dir", &fanout, e))?;
        let tmp = temp_sibling(&fanout, "chunk");
        fs::write(&tmp, bytes).map_err(|e| QxetError::io("write chunk", &tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| QxetError::io("rename chunk", &path, e))?;
        Ok(true)
    }

    /// Read one chunk and verify size and hash against its manifest
    /// entry. This is the verify-on-read path: any disagreement is a
    /// [`QxetError::CorruptChunk`] naming the chunk, never a silent
    /// substitution.
    fn read_chunk_verified(&self, chunk: &ChunkRef) -> Result<Vec<u8>> {
        let path = self.chunk_path(&chunk.hash);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                QxetError::MissingChunk(chunk.hash.clone())
            } else {
                QxetError::io("read chunk", &path, e)
            }
        })?;
        if bytes.len() as u64 != chunk.length {
            return Err(QxetError::CorruptChunk {
                hash: chunk.hash.clone(),
                reason: format!("size {} != manifest length {}", bytes.len(), chunk.length),
            });
        }
        let actual = sha256_hex(&bytes);
        if actual != chunk.hash {
            return Err(QxetError::CorruptChunk {
                hash: chunk.hash.clone(),
                reason: format!("content hashes to {actual}"),
            });
        }
        Ok(bytes)
    }

    /// Chunk and store the file at `path`, writing its manifest.
    ///
    /// Streams the file through the chunker with a single
    /// max-chunk-sized buffer; the whole file is never held in memory.
    /// The artifact name is the file's name component. Storing the
    /// same content under the same name twice is idempotent: the
    /// existing manifest's ID is returned and nothing is rewritten.
    ///
    /// # Errors
    /// Invalid name, unreadable file, too many chunks, I/O failures,
    /// or a pre-existing manifest that fails validation (the store
    /// must be repaired before new writes are trusted).
    pub fn put_file(&self, path: &Path) -> Result<PutOutcome> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| QxetError::InvalidName(path.display().to_string()))?
            .to_string();
        validate_artifact_name(&name)?;
        let file = fs::File::open(path).map_err(|e| QxetError::io("open source", path, e))?;
        if file
            .metadata()
            .map_err(|e| QxetError::io("stat source", path, e))?
            .is_dir()
        {
            return Err(QxetError::io(
                "open source",
                path,
                std::io::Error::new(std::io::ErrorKind::IsADirectory, "source is a directory"),
            ));
        }
        let mut reader = BufReader::with_capacity(CHUNK_MAX_BYTES, file);
        let mut read_buf = vec![0u8; CHUNK_MAX_BYTES];
        let mut pending: Vec<u8> = Vec::with_capacity(CHUNK_MAX_BYTES);
        let mut chunks: Vec<ChunkRef> = Vec::new();
        let mut new_chunk_count = 0usize;
        let mut offset: u64 = 0;
        let mut eof = false;
        loop {
            while !eof && pending.len() < CHUNK_MAX_BYTES {
                let space = CHUNK_MAX_BYTES - pending.len();
                match reader.read(&mut read_buf[..space]) {
                    Ok(0) => eof = true,
                    Ok(n) => pending.extend_from_slice(&read_buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(QxetError::io("read source", path, e)),
                }
            }
            if pending.is_empty() {
                break;
            }
            let cut_len = self.chunker.cut(&pending);
            assert!(
                cut_len >= 1 && cut_len <= pending.len(),
                "chunker contract violated"
            );
            let chunk_bytes: Vec<u8> = pending.drain(..cut_len).collect();
            let hash = sha256_hex(&chunk_bytes);
            if self.write_chunk(&hash, &chunk_bytes)? {
                new_chunk_count += 1;
            }
            chunks.push(ChunkRef {
                hash,
                length: cut_len as u64,
                offset,
            });
            offset = offset
                .checked_add(cut_len as u64)
                .ok_or_else(|| QxetError::TooLarge("file size overflows u64".to_string()))?;
            if chunks.len() > MAX_CHUNKS_PER_ARTIFACT {
                return Err(QxetError::TooLarge(format!(
                    "artifact exceeds {MAX_CHUNKS_PER_ARTIFACT} chunks"
                )));
            }
            if eof && pending.is_empty() {
                break;
            }
        }
        // Idempotence: an identical manifest (same name, size, chunk
        // list) may already exist under some timestamp; reuse its ID.
        for existing_id in self.manifest_ids()? {
            let existing = self.read_manifest(&existing_id)?;
            if existing.name == name && existing.size_bytes == offset && existing.chunks == chunks {
                return Ok(PutOutcome {
                    already_present: true,
                    chunk_count: chunks.len(),
                    id: existing_id,
                    name,
                    new_chunk_count,
                    size_bytes: offset,
                });
            }
        }
        let manifest = Manifest {
            chunks,
            created_unix_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            name: name.clone(),
            size_bytes: offset,
        };
        let id = manifest.content_id()?;
        let bytes = manifest.to_canonical_bytes()?;
        let manifests_dir = self.root.join("manifests");
        let tmp = temp_sibling(&manifests_dir, "manifest");
        fs::write(&tmp, &bytes).map_err(|e| QxetError::io("write manifest", &tmp, e))?;
        let target = self.manifest_path(&id);
        fs::rename(&tmp, &target).map_err(|e| QxetError::io("rename manifest", &target, e))?;
        Ok(PutOutcome {
            already_present: false,
            chunk_count: manifest.chunks.len(),
            id,
            name,
            new_chunk_count,
            size_bytes: offset,
        })
    }

    /// Reassemble the artifact `id` into `dest_dir` under its recorded
    /// name, verifying every chunk's hash as it is read. Output is
    /// written to a temp file in `dest_dir` and renamed over the final
    /// path only once the whole artifact verifies; a failed `get`
    /// leaves no partial file behind. Returns the output path.
    ///
    /// # Errors
    /// [`QxetError::InvalidId`], [`QxetError::NoSuchArtifact`],
    /// [`QxetError::MissingChunk`], [`QxetError::CorruptChunk`] naming
    /// the chunk, or I/O failures.
    pub fn get(&self, raw_id: &str, dest_dir: &Path) -> Result<PathBuf> {
        let id = normalize_id(raw_id)?;
        let manifest = self.read_manifest(&id)?;
        let out_path = dest_dir.join(&manifest.name);
        self.write_verified(&manifest, &out_path)?;
        Ok(out_path)
    }

    /// Like [`Store::get`], but writes to the exact file path
    /// `out_path` instead of the manifest's recorded name.
    ///
    /// # Errors
    /// Same as [`Store::get`].
    pub fn get_to(&self, raw_id: &str, out_path: &Path) -> Result<PathBuf> {
        let id = normalize_id(raw_id)?;
        let manifest = self.read_manifest(&id)?;
        self.write_verified(&manifest, out_path)?;
        Ok(out_path.to_path_buf())
    }

    /// Stream a manifest's chunks to `out_path`, verifying each one,
    /// via a temp file + rename so failures never leave partial or
    /// unverified output at the destination.
    fn write_verified(&self, manifest: &Manifest, out_path: &Path) -> Result<()> {
        let dest_dir = out_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| QxetError::Io {
                op: "resolve output dir",
                path: out_path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "output path has no parent",
                ),
            })?;
        let tmp = temp_sibling(&dest_dir, "get");
        let outcome = (|| -> Result<()> {
            let file =
                fs::File::create(&tmp).map_err(|e| QxetError::io("create output", &tmp, e))?;
            let mut writer = BufWriter::new(file);
            for chunk in &manifest.chunks {
                let bytes = self.read_chunk_verified(chunk)?;
                writer
                    .write_all(&bytes)
                    .map_err(|e| QxetError::io("write output", &tmp, e))?;
            }
            writer
                .flush()
                .map_err(|e| QxetError::io("write output", &tmp, e))?;
            writer
                .get_ref()
                .sync_all()
                .map_err(|e| QxetError::io("sync output", &tmp, e))?;
            Ok(())
        })();
        if let Err(e) = outcome {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        fs::rename(&tmp, out_path).map_err(|e| QxetError::io("rename output", out_path, e))?;
        Ok(())
    }

    /// Describe every artifact in the store, sorted by content ID.
    ///
    /// # Errors
    /// Any manifest that fails validation fails the listing: partial
    /// metadata presented as fact would be worse than an error.
    pub fn list(&self) -> Result<Vec<ArtifactInfo>> {
        let mut out = Vec::new();
        for id in self.manifest_ids()? {
            let manifest = self.read_manifest(&id)?;
            let mut seen = HashSet::new();
            let mut stored_bytes: u64 = 0;
            for chunk in &manifest.chunks {
                if seen.insert(&chunk.hash) {
                    stored_bytes += chunk.length;
                }
            }
            out.push(ArtifactInfo {
                chunk_count: manifest.chunks.len(),
                created_unix_secs: manifest.created_unix_secs,
                id,
                name: manifest.name,
                size_bytes: manifest.size_bytes,
                stored_bytes,
            });
        }
        Ok(out)
    }

    /// Rehash every chunk referenced by every manifest and report all
    /// corruption found. Unparseable manifests are reported, not
    /// fatal: the point is a complete damage report.
    pub fn verify(&self) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        let ids = self.manifest_ids()?;
        for id in ids {
            let manifest = match self.read_manifest(&id) {
                Ok(m) => m,
                Err(_) => {
                    report.corrupt_manifests.push(id);
                    continue;
                }
            };
            report.artifacts_checked += 1;
            for chunk in &manifest.chunks {
                report.chunks_checked += 1;
                match self.read_chunk_verified(chunk) {
                    Ok(_) => {}
                    Err(QxetError::MissingChunk(h))
                    | Err(QxetError::CorruptChunk { hash: h, .. }) => {
                        if !report.corrupt_chunks.contains(&h) {
                            report.corrupt_chunks.push(h);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        report.corrupt_chunks.sort();
        Ok(report)
    }

    /// Delete chunks no manifest references.
    ///
    /// With `dry_run = true` nothing is deleted; the report says what
    /// would go. Fails closed — refusing to delete anything — when any
    /// manifest cannot be parsed, because the mark set would be
    /// incomplete. Files under `chunks/` that are not chunk candidates
    /// (temp files from in-flight puts, stray files) are never touched.
    ///
    /// # Errors
    /// Any manifest parse/validation failure, or I/O failures.
    pub fn gc(&self, dry_run: bool) -> Result<GcReport> {
        let mut live: HashSet<String> = HashSet::new();
        for id in self.manifest_ids()? {
            let manifest = self.read_manifest(&id)?;
            for chunk in &manifest.chunks {
                live.insert(chunk.hash.clone());
            }
        }
        let mut report = GcReport {
            dry_run,
            ..GcReport::default()
        };
        let chunks_dir = self.root.join("chunks");
        let fanouts = fs::read_dir(&chunks_dir)
            .map_err(|e| QxetError::io("list chunk dirs", &chunks_dir, e))?;
        for fanout in fanouts {
            let fanout = fanout.map_err(|e| QxetError::io("list chunk dirs", &chunks_dir, e))?;
            if !fanout.path().is_dir() {
                report.skipped_file_count += 1;
                continue;
            }
            let entries = fs::read_dir(fanout.path())
                .map_err(|e| QxetError::io("list chunks", &fanout.path(), e))?;
            for entry in entries {
                let entry = entry.map_err(|e| QxetError::io("list chunks", &fanout.path(), e))?;
                let path = entry.path();
                let Some(fname) = entry.file_name().into_string().ok() else {
                    report.skipped_file_count += 1;
                    continue;
                };
                if !is_hex_id(&fname) || live.contains(&fname) {
                    if !is_hex_id(&fname) {
                        report.skipped_file_count += 1;
                    }
                    continue;
                }
                let size = entry
                    .metadata()
                    .map_err(|e| QxetError::io("stat chunk", &path, e))?
                    .len();
                report.orphan_chunk_count += 1;
                report.orphan_bytes += size;
                if !dry_run {
                    fs::remove_file(&path).map_err(|e| QxetError::io("remove chunk", &path, e))?;
                    report.removed_chunk_count += 1;
                    report.removed_bytes += size;
                }
            }
        }
        Ok(report)
    }
}
