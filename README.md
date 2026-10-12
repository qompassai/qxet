# qxet

QXet: Quality AI eXchange — Qompass AI's Rust work on the Xet
content-addressed storage protocol (the deduplicated chunk store behind
Hugging Face Hub file transfers).

## Repository state: fragments, not yet a buildable crate

This repo currently holds two source fragments salvaged from a larger
Rust workspace (Hugging Face's `hf_xet` Python bindings and the `data`
crate's `xtool` migration binary). The rest of that workspace was never
committed here, so **the code does not compile and is not expected to**
until the missing pieces below are restored or rewritten. The
`Cargo.toml` files in this repo are a scaffold that records the intended
workspace shape; they are marked as such.

### What is present

| File | What it is |
| --- | --- |
| `hf_xet/src/lib.rs` | pyo3 bindings crate root: `upload_files` / `download_files` Python functions and the `PyPointerFile` class |
| `data/src/bin/xtool.rs` | CLI for dedup analysis and migration of files into a Xet CAS (`dedup`, `query` subcommands) |

### What is missing (specific)

`hf_xet` crate modules referenced by `lib.rs` but absent:
`log`, `log_buffer`, `progress_update`, `runtime` (`async_run`,
`init_threadpool`), `token_refresh` (`WrappedTokenRefresher`), and the
feature-gated `profiling` module.

Workspace crates referenced but absent:
- `data` library target — `errors::DataProcessingError`,
  `data_client::{upload_async, download_async}`, `PointerFile`, and
  `migration_tool::{hub_client::HubClient, migrate::migrate_files_impl}`
- `cas_object` — `CompressionScheme`
- `xet_threadpool` — `ThreadPool`
- `utils` — `progress::ProgressUpdater`

External crates the fragments use: `pyo3` (hf_xet), `anyhow`, `clap`,
`serde_json`, `walkdir` (xtool).

### Path to a build

1. Restore or rewrite the four missing workspace crates above,
   signatures as pinned by the two fragments' call sites.
2. Restore the five missing `hf_xet` modules.
3. Remove the SCAFFOLD banners from the manifests, then `cargo build`.

Until then the fragments are reference material: they pin the intended
API surface (function signatures, Python bindings shape, CLI surface)
for whoever does the restoration.

## Memory

Project memory lives in `.ai/memory/` (see `AGENTS.md` for the protocol).
