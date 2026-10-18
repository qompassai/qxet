# qxet

QXet: Quality AI eXchange — content-addressed storage for large binary
artifacts (model weights, datasets, checkpoints). Files are chunked
with FastCDC content-defined chunking, every chunk is addressed by its
SHA-256, identical chunks are stored once across all artifacts, and
every read verifies hashes and fails closed on any mismatch. Think
git semantics for large binary blobs — local-first, no network.

<a href="./LICENSE"><img src="https://img.shields.io/badge/License-Apache%202.0-blue.svg" alt="License: Apache 2.0"></a>

**Status: v0.1.0** — the local store and CLI described in
[DESIGN.md](./DESIGN.md) are implemented and gated (fmt, strict
clippy, 39 tests including FastCDC known-answer vectors and a
corruption suite). Remotes, the Hub protocol, and Python bindings are
v0.2 and do not exist yet.

## Usage

```console
$ qxet init
initialized store at ./.qxet

$ qxet put weights-v1.bin
qxet:8f37d3aab7c317186a90d36e88a7ff5d208d0857656ba1edb2d6182aac3b9dc3
stored weights-v1.bin: 300000 bytes, 3 chunks (3 new)

$ qxet put weights-v2.bin        # weights-v1 + 50 KB appended
qxet:e9a499430800f57923948a557fac49fb1fe51e647223038b1110c395e4625e98
stored weights-v2.bin: 350000 bytes, 4 chunks (2 new)

$ qxet list
qxet:8f37...  weights-v1.bin  size=300000 chunks=3 stored=300000 dedup=1.00x created=...
qxet:e9a4...  weights-v2.bin  size=350000 chunks=4 stored=350000 dedup=1.00x created=...

$ qxet get qxet:e9a499430800f57923948a557fac49fb1fe51e647223038b1110c395e4625e98
wrote ./weights-v2.bin

$ qxet verify
checked 2 artifact(s), 7 chunk(s)
store OK

$ qxet gc            # dry run by default; --apply deletes
dry run: 0 orphan chunk(s), 0 bytes reclaimable (0 file(s) skipped); re-run with --apply to delete
```

The two versions above occupy 5 chunk objects (403 KB) for 650 KB of
artifacts, because content-defined boundaries keep the shared prefix
in shared chunks. `put` of identical content under the same name is
idempotent. `get` accepts the `qxet:` prefix or bare hex, supports
`-o <path>` for an explicit output file, and locates the store by
walking up from the current directory (override with `--store`).

## Store layout

```text
.qxet/
  chunks/ab/cdef...    # chunk bytes, named by SHA-256, 2-hex fan-out
  manifests/<id>.json  # canonical JSON manifests, named by content ID
  config.toml          # chunking parameters the store was built with
```

A manifest records the artifact name, size, creation time, and the
ordered chunk list (hash + offset + length). The manifest ID is the
SHA-256 of its canonical JSON, so manifests are tamper-evident too.
Chunking parameters: FastCDC, min 16 KiB / avg 64 KiB / max 256 KiB,
recorded in `config.toml`; a store refuses to open under a build with
different parameters.

## Integrity model

- `get` rehashes every chunk as it reads and aborts — naming the
  chunk — on any size or hash mismatch. Output goes to a temp file
  and is renamed into place only after the whole artifact verifies;
  a failed `get` leaves nothing behind.
- `verify` walks every manifest and rehashes every chunk, reporting
  all corruption it finds.
- `gc` refuses to run if any manifest fails to parse: with an
  incomplete mark set it could delete live chunks. Temp files and
  stray files under `chunks/` are never touched.
- Manifests are parsed as untrusted input: traversal names, zero
  lengths, offset gaps, and size mismatches are all rejected before
  any chunk is read.

## Repository layout

```text
crates/qxet-core/   # chunking (FastCDC), CAS store, manifest, verify, gc
crates/qxet-cli/    # the `qxet` binary (clap, thin over qxet-core)
hf_xet/, data/       # reference fragments from an earlier workspace;
                     # not part of the v0.1 build (see below)
DESIGN.md            # the design of record
```

The `hf_xet` (pyo3 bindings) and `data`/`xtool` fragments are kept as
reference material: they pin the API surface an earlier restoration
plan aimed at, and `hf_xet`'s shape informs the v0.2 Python bindings.
They are excluded from the workspace and do not compile; nothing in
v0.1 depends on them.

## Development

Pinned toolchain: stable Rust 1.99.0 (`rust-toolchain.toml`).

```console
$ cargo fmt --check
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo test --workspace
```

Tests split roughly half validation / half adversarial, per the
design: round-trips, dedup accounting, and GC on the happy side;
flipped chunk bytes, truncated manifests, re-signed tampered
manifests, traversal names, hostile IDs, and concurrent puts on the
other.

## Memory

Project memory lives in `.ai/memory/` (see `AGENTS.md` for the
protocol).

## License

This project is licensed under the [Apache License, Version 2.0](./LICENSE).
