# qxet — Quality AI eXchange: Greenfield Design

**Status:** Draft for build (2026-10-05). Owner approved greenfield scope.

## Vision

Content-addressed storage and exchange for AI model artifacts. Model weights
are large (GBs), highly redundant across versions (a fine-tune differs from
its base model in a fraction of bytes), and integrity-critical. Git chokes on
them; tarballs lose provenance; HuggingFace Hub locks you into their infra.

qxet gives you git-like semantics for large binary artifacts: chunk it once,
address everything by hash, deduplicate across versions, verify on every
read, move artifacts between machines.

The name keeps Qompass's "Quality" prefix (cf. qonfig = Quality dotfile
manager). The xet connection is conceptual: content-defined chunking +
content-addressed storage, as pioneered by HuggingFace's xet-core — but as an
independent, quality-focused implementation, not a fork.

## MVP: v0.1.0 (this build)

A working local CAS with a CLI. No network, no Hub protocol — those are v0.2.

### Commands

| Command | Behavior |
|---|---|
| `qxet init` | Create `.qxet/` store in cwd (chunks/, manifests/, config). |
| `qxet put <file>` | Chunk with FastCDC, SHA-256 each chunk, store dedup'd, write manifest, print content ID (`qxet:<sha256>`). |
| `qxet get <id> [-o <path>]` | Reassemble from manifest, verify every chunk hash, write file. Fails closed on any mismatch. |
| `qxet list` | Artifacts: content ID, original name/size, chunk count, stored bytes, dedup ratio. |
| `qxet verify` | Walk all manifests, rehash every chunk, report corruption. |
| `qxet gc` | Remove chunks unreferenced by any manifest (with `--dry-run` default-safe). |

### Architecture

```
crates/
  qxet-core/      # chunking (FastCDC), CAS store, manifest model, verify
  qxet-cli/       # clap CLI, thin over qxet-core
```

- **Chunking:** FastCDC content-defined chunking (avg 64 KiB, min 16 KiB,
  max 256 KiB). Content-defined boundaries mean a 1-byte insertion shifts
  only local chunks — the dedup win for fine-tunes.
- **Hashing:** SHA-256 throughout. Chunk IDs are chunk hashes; manifest ID
  is the hash of the canonical manifest.
- **Manifest:** JSON (canonical key order): artifact name, size, chunk list
  (hash + offset + length), created timestamp. Manifest itself
  content-addressed.
- **Store layout:**
  ```
  .qxet/
    chunks/ab/cd...   # first 2 hex chars as fan-out dir
    manifests/<id>.json
    config.toml
  ```
- **Integrity:** `get` and `verify` rehash everything. No trust without
  verification — a corrupt chunk fails the operation, never silently.

### Tiger Style Rust

Follows `tiger-style-rust`: safety > performance > DX. Explicit contracts
on every public function (inputs, bounds, failure modes). Bounded work:
chunk sizes, manifest sizes, file sizes all have named limits. No `unwrap`
on I/O paths — `anyhow`/`thiserror` with context. Half the tests
adversarial: corrupt chunks, truncated manifests, hash mismatches, empty
files, 0-byte chunks, concurrent puts.

### v0.2 (out of scope for this build, documented)

- `qxet push/pull <remote>`: transfer manifests + missing chunks over
  HTTPS to a dumb remote (content-addressed HTTP store — no server logic
  needed beyond PUT/GET by hash).
- Python bindings (`pyo3`) — the old `hf_xet` fragment's shape, done right.
- HuggingFace Hub interoperability (read xet-backed repos).

## Build plan

1. Scaffold workspace (`Cargo.toml` workspace, `qxet-core`, `qxet-cli`).
2. Implement FastCDC chunking + tests (known-answer vectors).
3. Implement CAS store (put/get/list) + tests.
4. Implement manifest model + verify + gc.
5. CLI wiring with clap; integration tests (assert_cmd or manual).
6. `--update-memory` + handoff via bloch; docs (README with real usage).
7. Build + clippy + tests green on primo (nightly per phlow rules? No —
   qxet is its own project; stable Rust unless Owner says otherwise).

## Open questions for Owner (non-blocking)

- Remote protocol in v0.2: dumb HTTP store vs. something smarter?
- Python bindings priority: v0.2 or later?
- License: keep AGPL+QCDA dual (repo already has both)?
