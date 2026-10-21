# 2026-10-13 — License settled: Apache-2.0

- Owner ruled Apache-2.0 (2026-10-13), closing the open license question
  recorded in the 2026-10-11 overnight-pass inbox note and in
  `current.md` ("License state is unresolved... Awaiting Owner's call").
- Landed in one commit: `LICENSE` added (canonical Apache-2.0 text,
  appendix `Copyright 2025 Qompass AI`, byte-identical to the normalized
  qompassai language repos); `CITATION.cff` license `Q-CDA-1.0` →
  `Apache-2.0`; `.zenodo.json` license `Q-CDA-1.0` → `Apache-2.0`;
  README gained the Apache-2.0 badge and a License section.
- No manifest changes were needed: the workspace `Cargo.toml` already
  declared `license = "Apache-2.0"` and both member crates already use
  `license.workspace = true`.
- `current.md` still describes the license state as unresolved; this
  note is the record pending promotion into `current.md`/`decisions.md`.
