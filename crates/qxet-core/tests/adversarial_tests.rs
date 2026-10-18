//! Adversarial tests: corrupt, truncated, tampered, and hostile
//! store contents. The store's contract is to fail closed and name
//! the offending chunk or manifest — never to panic, never to emit
//! unverified bytes, and never to write outside the target directory.

use std::fs;
use std::path::PathBuf;

use qxet_core::error::QxetError;
use qxet_core::hash::sha256_hex;
use qxet_core::manifest::{ChunkRef, Manifest};
use qxet_core::store::Store;

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

struct Fixture {
    _tmp: tempfile::TempDir,
    out_dir: PathBuf,
    src_dir: PathBuf,
    store: Store,
    artifact_id: String,
    manifest: Manifest,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src_dir = tmp.path().join("src");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&src_dir).expect("src dir");
    fs::create_dir_all(&out_dir).expect("out dir");
    let store = Store::init(tmp.path()).expect("init");
    let bytes = pseudo_random(400_000, 71);
    let src = src_dir.join("victim.bin");
    fs::write(&src, &bytes).expect("write source");
    let outcome = store.put_file(&src).expect("put");
    let mpath = store
        .root()
        .join("manifests")
        .join(format!("{}.json", outcome.id));
    let manifest = Manifest::parse(&outcome.id, &fs::read(&mpath).expect("manifest bytes"))
        .expect("manifest parses");
    Fixture {
        _tmp: tmp,
        out_dir,
        src_dir,
        store,
        artifact_id: outcome.id,
        manifest,
    }
}

fn chunk_path(store: &Store, hash: &str) -> PathBuf {
    store.root().join("chunks").join(&hash[..2]).join(hash)
}

fn manifest_path(store: &Store, id: &str) -> PathBuf {
    store.root().join("manifests").join(format!("{id}.json"))
}

/// Replace a manifest with a re-signed tampered copy: writes the
/// manifest under its new content ID and removes the old file, as an
/// attacker with store write access would have to.
fn resign(fx: &Fixture, manifest: &Manifest) -> String {
    let new_id = manifest.content_id().expect("content id");
    let bytes = manifest.to_canonical_bytes().expect("canonical");
    fs::write(manifest_path(&fx.store, &new_id), bytes).expect("write tampered manifest");
    fs::remove_file(manifest_path(&fx.store, &fx.artifact_id)).expect("remove original");
    new_id
}

#[test]
fn flipped_chunk_byte_fails_get_and_verify_naming_the_chunk() {
    let fx = fixture();
    let victim = &fx.manifest.chunks[0];
    let path = chunk_path(&fx.store, &victim.hash);
    let mut bytes = fs::read(&path).expect("chunk bytes");
    bytes[0] ^= 0xFF;
    fs::write(&path, &bytes).expect("corrupt chunk");
    let err = fx
        .store
        .get(&fx.artifact_id, &fx.out_dir)
        .expect_err("get must fail");
    assert!(
        matches!(&err, QxetError::CorruptChunk { hash, .. } if hash == &victim.hash),
        "wrong error: {err}"
    );
    // Fail closed: no output file, no temp litter in the output dir.
    assert!(!fx.out_dir.join("victim.bin").exists());
    assert_eq!(fs::read_dir(&fx.out_dir).expect("out dir").count(), 0);
    let report = fx.store.verify().expect("verify");
    assert_eq!(report.corrupt_chunks, vec![victim.hash.clone()]);
}

#[test]
fn truncated_chunk_file_fails_get() {
    let fx = fixture();
    let victim = &fx.manifest.chunks[1];
    let path = chunk_path(&fx.store, &victim.hash);
    let bytes = fs::read(&path).expect("chunk bytes");
    fs::write(&path, &bytes[..bytes.len() / 2]).expect("truncate chunk");
    let err = fx
        .store
        .get(&fx.artifact_id, &fx.out_dir)
        .expect_err("get must fail");
    assert!(
        matches!(&err, QxetError::CorruptChunk { hash, .. } if hash == &victim.hash),
        "{err}"
    );
}

#[test]
fn missing_chunk_fails_get_and_verify() {
    let fx = fixture();
    let victim = &fx.manifest.chunks[2];
    fs::remove_file(chunk_path(&fx.store, &victim.hash)).expect("remove chunk");
    let err = fx
        .store
        .get(&fx.artifact_id, &fx.out_dir)
        .expect_err("get must fail");
    assert!(
        matches!(&err, QxetError::MissingChunk(h) if h == &victim.hash),
        "{err}"
    );
    let report = fx.store.verify().expect("verify");
    assert_eq!(report.corrupt_chunks, vec![victim.hash.clone()]);
}

#[test]
fn truncated_manifest_fails_loudly_everywhere() {
    let fx = fixture();
    let path = manifest_path(&fx.store, &fx.artifact_id);
    let bytes = fs::read(&path).expect("manifest");
    fs::write(&path, &bytes[..bytes.len() / 2]).expect("truncate manifest");
    assert!(fx.store.get(&fx.artifact_id, &fx.out_dir).is_err());
    assert!(fx.store.list().is_err());
    // gc must refuse: a partial mark set could delete live chunks.
    assert!(fx.store.gc(false).is_err());
    // verify reports rather than dying.
    let report = fx.store.verify().expect("verify");
    assert_eq!(report.corrupt_manifests, vec![fx.artifact_id.clone()]);
}

#[test]
fn manifest_under_wrong_id_is_rejected() {
    let fx = fixture();
    let wrong = "f".repeat(64);
    fs::rename(
        manifest_path(&fx.store, &fx.artifact_id),
        manifest_path(&fx.store, &wrong),
    )
    .expect("rename manifest");
    let err = fx
        .store
        .get(&wrong, &fx.out_dir)
        .expect_err("get must fail");
    assert!(matches!(err, QxetError::CorruptManifest { .. }), "{err}");
}

#[test]
fn tampered_manifest_pointing_at_missing_chunk_fails() {
    let fx = fixture();
    let mut tampered = fx.manifest.clone();
    tampered.chunks[0].hash = "e".repeat(64);
    let new_id = resign(&fx, &tampered);
    let err = fx
        .store
        .get(&new_id, &fx.out_dir)
        .expect_err("get must fail");
    assert!(
        matches!(&err, QxetError::MissingChunk(h) if h == &"e".repeat(64)),
        "{err}"
    );
}

#[test]
fn traversal_name_in_manifest_cannot_escape_output_dir() {
    let fx = fixture();
    let mut tampered = fx.manifest.clone();
    tampered.name = "../evil.bin".to_string();
    let new_id = resign(&fx, &tampered);
    assert!(fx.store.get(&new_id, &fx.out_dir).is_err());
    assert!(!fx._tmp.path().join("evil.bin").exists());
    assert_eq!(fs::read_dir(&fx.out_dir).expect("out dir").count(), 0);
}

#[test]
fn zero_length_chunk_in_manifest_is_rejected() {
    let fx = fixture();
    let mut tampered = fx.manifest.clone();
    tampered.chunks.insert(
        0,
        ChunkRef {
            hash: "a".repeat(64),
            length: 0,
            offset: 0,
        },
    );
    // Shift the real chunks so offsets tile — isolates the zero
    // length as the only defect.
    let mut off = 0u64;
    for c in &mut tampered.chunks {
        c.offset = off;
        off += c.length;
    }
    let new_id = resign(&fx, &tampered);
    assert!(fx.store.get(&new_id, &fx.out_dir).is_err());
}

#[test]
fn size_sum_mismatch_in_manifest_is_rejected() {
    let fx = fixture();
    let mut tampered = fx.manifest.clone();
    tampered.size_bytes += 1;
    let new_id = resign(&fx, &tampered);
    assert!(fx.store.get(&new_id, &fx.out_dir).is_err());
}

#[test]
fn hostile_ids_are_rejected_before_any_path_is_built() {
    let fx = fixture();
    for bad in [
        "../chunks",
        "..",
        "a/b",
        &"z".repeat(64),
        &"a".repeat(63),
        "qxet:../x",
        "",
    ] {
        let err = fx
            .store
            .get(bad, &fx.out_dir)
            .expect_err("id must be rejected");
        assert!(matches!(err, QxetError::InvalidId(_)), "{bad:?} gave {err}");
    }
    assert_eq!(fs::read_dir(&fx.out_dir).expect("out dir").count(), 0);
}

#[test]
fn put_of_directory_and_missing_file_are_clean_errors() {
    let fx = fixture();
    assert!(fx.store.put_file(&fx.src_dir).is_err());
    assert!(fx.store.put_file(&fx.src_dir.join("nope.bin")).is_err());
}

#[test]
fn tampered_store_config_is_rejected() {
    let fx = fixture();
    let config = fx.store.root().join("config.toml");
    let mut text = fs::read_to_string(&config).expect("config");
    text = text.replace("65536", "32768");
    fs::write(&config, text).expect("tamper config");
    let err = Store::open(fx.store.root()).expect_err("open must fail");
    assert!(matches!(err, QxetError::ConfigMismatch(_)), "{err}");
}

#[test]
fn hand_built_forged_store_cannot_pass_verify() {
    // An attacker assembles a manifest + chunk files from scratch:
    // chunk content does not match the hash it is filed under.
    let fx = fixture();
    let forged = b"forged weights".to_vec();
    let claimed = sha256_hex(b"the real weights");
    let path = chunk_path(&fx.store, &claimed);
    fs::create_dir_all(path.parent().expect("fanout")).expect("fanout");
    fs::write(&path, &forged).expect("write forged chunk");
    let manifest = Manifest {
        chunks: vec![ChunkRef {
            hash: claimed.clone(),
            length: forged.len() as u64,
            offset: 0,
        }],
        created_unix_secs: 1,
        name: "forged.bin".to_string(),
        size_bytes: forged.len() as u64,
    };
    let id = resign(&fx, &manifest);
    let report = fx.store.verify().expect("verify");
    assert!(report.corrupt_chunks.contains(&claimed), "{report:?}");
    assert!(fx.store.get(&id, &fx.out_dir).is_err());
}
