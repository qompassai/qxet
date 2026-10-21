# Current Work

qxet (QXet: Quality AI eXchange) — Qompass AI's content-addressed
storage for large binary artifacts.

## Status

- v0.1.0 implemented 2026-10-17 per DESIGN.md: `crates/qxet-core`
  (FastCDC chunking 16/64/256 KiB, SHA-256 CAS, canonical JSON
  manifests, verify-on-read, gc) and `crates/qxet-cli`
  (`init`/`put`/`get`/`list`/`verify`/`gc`). Gates on primo
  (stable 1.99.0, pinned in `rust-toolchain.toml`): fmt clean,
  strict clippy clean, 39 tests passing, release build green.
- The `hf_xet/` and `data/` fragments are kept as reference material
  and excluded from the workspace (`exclude` in the root manifest);
  they still do not compile and nothing in v0.1 depends on them.
  README.md documents their role.
- License is settled: Apache-2.0. `LICENSE` is present and
  `.zenodo.json` / `CITATION.cff` agree. The earlier "license
  unresolved" note is retired.
- v0.2 (remotes, Hub protocol, Python bindings) is unstarted; the
  open questions in DESIGN.md remain Owner's calls.

## Standing rules

- Code changes are pushed only under Owner's explicit direction
  (the 2026-10-17 night authorization covered the v0.1 landing).
