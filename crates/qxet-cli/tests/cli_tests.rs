//! End-to-end tests of the `qxet` binary: real process invocations
//! in a temp working directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn qxet(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qxet"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run qxet")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.max(1);
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let v = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn chunk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for fanout in fs::read_dir(root.join(".qxet/chunks"))
        .expect("fanouts")
        .flatten()
    {
        if fanout.path().is_dir() {
            for entry in fs::read_dir(fanout.path()).expect("entries").flatten() {
                out.push(entry.path());
            }
        }
    }
    out
}

#[test]
fn cli_full_workflow() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let bytes = pseudo_random(700_000, 81);
    fs::write(dir.join("model.bin"), &bytes).expect("write model");

    let out = qxet(dir, &["init"]);
    assert!(out.status.success(), "init: {}", stdout(&out));
    assert!(dir.join(".qxet/config.toml").is_file());

    let out = qxet(dir, &["put", "model.bin"]);
    assert!(out.status.success(), "put: {}", stdout(&out));
    let text = stdout(&out);
    let id_line = text.lines().next().expect("id line").to_string();
    assert!(id_line.starts_with("qxet:"), "first line is the id: {text}");

    // get with default output name in a subdirectory (store discovery
    // walks up), then byte-compare.
    let sub = dir.join("elsewhere");
    fs::create_dir_all(&sub).expect("subdir");
    let out = qxet(&sub, &["get", &id_line]);
    assert!(out.status.success(), "get: {}", stdout(&out));
    assert_eq!(fs::read(sub.join("model.bin")).expect("read"), bytes);

    let out = qxet(dir, &["list"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("model.bin"), "list: {}", stdout(&out));

    let out = qxet(dir, &["verify"]);
    assert!(out.status.success(), "verify: {}", stdout(&out));
    assert!(stdout(&out).contains("store OK"));

    // gc defaults to a dry run: exit 0, nothing deleted.
    let before = chunk_files(dir).len();
    let out = qxet(dir, &["gc"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("dry run"), "gc: {}", stdout(&out));
    assert_eq!(chunk_files(dir).len(), before);
    // The explicit --dry-run spelling works too, and conflicts with
    // --apply.
    let out = qxet(dir, &["gc", "--dry-run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("dry run"));
    let out = qxet(dir, &["gc", "--apply", "--dry-run"]);
    assert!(!out.status.success(), "--apply + --dry-run must conflict");
}

#[test]
fn cli_get_corrupt_chunk_exits_nonzero_and_writes_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    fs::write(dir.join("m.bin"), pseudo_random(300_000, 82)).expect("write");
    assert!(qxet(dir, &["init"]).status.success());
    let out = qxet(dir, &["put", "m.bin"]);
    let id_line = stdout(&out).lines().next().expect("id").to_string();

    // Corrupt one chunk byte on disk.
    let chunk = chunk_files(dir).into_iter().next().expect("a chunk");
    let mut bytes = fs::read(&chunk).expect("chunk bytes");
    bytes[3] ^= 0x01;
    fs::write(&chunk, &bytes).expect("corrupt");

    let dest = dir.join("should-not-exist.bin");
    let out = qxet(dir, &["get", &id_line, "-o", dest.to_str().expect("utf8")]);
    assert!(!out.status.success(), "corrupt get must fail");
    assert!(!dest.exists(), "failed get left an output file");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("corrupt chunk"),
        "stderr names the chunk: {stderr}"
    );

    let out = qxet(dir, &["verify"]);
    assert!(!out.status.success(), "verify must fail on corruption");
    assert!(stdout(&out).contains("corrupt chunk"));
}

#[test]
fn cli_rejects_bad_ids_and_missing_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    // No store anywhere: get fails with a helpful message.
    let out = qxet(dir, &["get", &"a".repeat(64)]);
    assert!(!out.status.success());
    assert!(qxet(dir, &["init"]).status.success());
    for bad in ["../etc/passwd", "xyz", &"a".repeat(63)] {
        let out = qxet(dir, &["get", bad]);
        assert!(!out.status.success(), "accepted bad id {bad:?}");
    }
    // Well-formed but absent artifact.
    let out = qxet(dir, &["get", &"b".repeat(64)]);
    assert!(!out.status.success());
}
