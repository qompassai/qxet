# Current Work

qxet (QXet: Quality AI eXchange) — Qompass AI's Rust work on the Xet
content-addressed storage protocol.

## Status

- Fragment state: only `hf_xet/src/lib.rs` and `data/src/bin/xtool.rs`
  are present; the workspace crates and modules they call are missing,
  so the project does not build yet. README.md carries the exact
  missing-pieces inventory and the path to a build.
- `Cargo.toml` files are a marked scaffold of the intended workspace.
- License state is unresolved: no LICENSE file in repo; `.zenodo.json`
  says Q-CDA-1.0, decisions.md records org Apache-2.0 normalization.
  Awaiting Matt's call.

## Standing rules

- Do not present the fragments as buildable; keep the SCAFFOLD banners
  until a real `cargo build` passes on primo.
- Code changes are pushed only under Matt's explicit direction
  (the 2026-10-11 overnight campaign authorization covered this pass).
