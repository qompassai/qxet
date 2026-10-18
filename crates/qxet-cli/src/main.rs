//! qxet command-line interface: a thin layer over qxet-core.
//!
//! Commands (DESIGN.md v0.1): `init`, `put`, `get`, `list`, `verify`,
//! `gc`. Every error from the core is printed to stderr and exits
//! non-zero; a corrupt store reported by `verify` also exits non-zero.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use qxet_core::error::QxetError;
use qxet_core::store::Store;

#[derive(Parser)]
#[command(
    name = "qxet",
    version,
    about = "Content-addressed store for large binary artifacts"
)]
struct Cli {
    /// Path to a `.qxet` store directory, or to a directory to search
    /// upward from. Default: discover from the current directory.
    #[arg(long, global = true)]
    store: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Collect chunks no manifest references. Safe by default: only
    /// reports. Pass --apply to actually delete.
    Gc {
        /// Actually delete orphan chunks (default is a dry run).
        #[arg(long, conflicts_with = "dry_run")]
        apply: bool,
        /// Report what would be removed without deleting (the
        /// default; the flag exists for scripts that want to say so).
        #[arg(long)]
        dry_run: bool,
    },
    /// Reassemble an artifact, verifying every chunk hash.
    Get {
        /// Content ID (`qxet:<sha256>` or bare hex).
        id: String,
        /// Output file path. Default: the artifact's recorded name in
        /// the current directory.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Create a `.qxet/` store in the given directory (default:
    /// current directory).
    Init {
        /// Directory to create the store in.
        dir: Option<PathBuf>,
    },
    /// List all artifacts in the store.
    List,
    /// Chunk, deduplicate, and store a file; prints its content ID.
    Put {
        /// File to store.
        file: PathBuf,
    },
    /// Rehash every chunk in the store and report corruption.
    Verify,
}

fn open_store(store_arg: &Option<PathBuf>) -> Result<Store, QxetError> {
    match store_arg {
        Some(path) if path.join("config.toml").is_file() => Store::open(path),
        Some(path) => Store::discover(path),
        None => Store::discover(Path::new(".")),
    }
}

fn run(cli: Cli) -> Result<(), QxetError> {
    match &cli.command {
        Command::Init { dir } => {
            let base = dir.clone().unwrap_or_else(|| PathBuf::from("."));
            let store = Store::init(&base)?;
            println!("initialized store at {}", store.root().display());
            Ok(())
        }
        Command::Put { file } => {
            let store = open_store(&cli.store)?;
            let outcome = store.put_file(file)?;
            println!("qxet:{}", outcome.id);
            if outcome.already_present {
                println!(
                    "already present: {} ({} bytes, {} chunks)",
                    outcome.name, outcome.size_bytes, outcome.chunk_count
                );
            } else {
                println!(
                    "stored {}: {} bytes, {} chunks ({} new)",
                    outcome.name, outcome.size_bytes, outcome.chunk_count, outcome.new_chunk_count
                );
            }
            Ok(())
        }
        Command::Get { id, out } => {
            let store = open_store(&cli.store)?;
            let cwd = PathBuf::from(".");
            let written = match out {
                Some(path) => store.get_to(id, path)?,
                None => store.get(id, &cwd)?,
            };
            println!("wrote {}", written.display());
            Ok(())
        }
        Command::List => {
            let store = open_store(&cli.store)?;
            let artifacts = store.list()?;
            for a in &artifacts {
                let ratio = if a.stored_bytes == 0 {
                    1.0
                } else {
                    a.size_bytes as f64 / a.stored_bytes as f64
                };
                println!(
                    "qxet:{}  {}  size={} chunks={} stored={} dedup={:.2}x created={}",
                    a.id,
                    a.name,
                    a.size_bytes,
                    a.chunk_count,
                    a.stored_bytes,
                    ratio,
                    a.created_unix_secs
                );
            }
            println!("{} artifact(s)", artifacts.len());
            Ok(())
        }
        Command::Verify => {
            let store = open_store(&cli.store)?;
            let report = store.verify()?;
            println!(
                "checked {} artifact(s), {} chunk(s)",
                report.artifacts_checked, report.chunks_checked
            );
            for id in &report.corrupt_manifests {
                println!("corrupt manifest: {id}");
            }
            for hash in &report.corrupt_chunks {
                println!("corrupt chunk: {hash}");
            }
            if report.is_clean() {
                println!("store OK");
                Ok(())
            } else {
                Err(QxetError::CorruptManifest {
                    id: String::new(),
                    reason: format!(
                        "{} corrupt chunk(s), {} corrupt manifest(s)",
                        report.corrupt_chunks.len(),
                        report.corrupt_manifests.len()
                    ),
                })
            }
        }
        Command::Gc { apply, dry_run: _ } => {
            let store = open_store(&cli.store)?;
            let report = store.gc(!apply)?;
            if *apply {
                println!(
                    "removed {} chunk(s), {} bytes reclaimed ({} file(s) skipped)",
                    report.removed_chunk_count, report.removed_bytes, report.skipped_file_count
                );
            } else {
                println!(
                    "dry run: {} orphan chunk(s), {} bytes reclaimable ({} file(s) skipped); \
                     re-run with --apply to delete",
                    report.orphan_chunk_count, report.orphan_bytes, report.skipped_file_count
                );
            }
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("qxet: {e}");
            ExitCode::FAILURE
        }
    }
}
