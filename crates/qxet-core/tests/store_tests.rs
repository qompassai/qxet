//! Validation tests: the store's happy paths, exercised end to end
//! against real files in a temp directory.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use qxet_core::manifest::Manifest;
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
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src_dir = tmp.path().join("src");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&src_dir).expect("src dir");
    fs::create_dir_all(&out_dir).expect("out dir");
    let store = Store::init(tmp.path()).expect("init store");
    Fixture {
        _tmp: tmp,
        out_dir,
        src_dir,
        store,
    }
}

fn write_src(fx: &Fixture, name: &str, bytes: &[u8]) -> PathBuf {
    let path = fx.src_dir.join(name);
    fs::write(&path, bytes).expect("write source");
    path
}

fn chunk_files(store: &Store) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let chunks = store.root().join("chunks");
    for fanout in fs::read_dir(&chunks).expect("fanouts").flatten() {
        if fanout.path().is_dir() {
            for entry in fs::read_dir(fanout.path()).expect("entries").flatten() {
                out.push(entry.path());
            }
        }
    }
    out
}

fn read_manifest(store: &Store, id: &str) -> Manifest {
    let path = store.root().join("manifests").join(format!("{id}.json"));
    let bytes = fs::read(&path).expect("manifest file");
    Manifest::parse(id, &bytes).expect("manifest parses")
}

#[test]
fn init_creates_design_layout() {
    let fx = fixture();
    let root = fx.store.root();
    assert!(root.join("chunks").is_dir());
    assert!(root.join("manifests").is_dir());
    assert!(root.join("config.toml").is_file());
    // Re-init is idempotent.
    let again = Store::init(fx._tmp.path()).expect("re-init");
    assert_eq!(again.root(), root);
}

#[test]
fn round_trip_many_shapes() {
    let fx = fixture();
    let shapes: Vec<(&str, Vec<u8>)> = vec![
        ("empty.bin", vec![]),
        ("one.bin", vec![0x42]),
        (
            "exactly-min.bin",
            pseudo_random(qxet_core::CHUNK_MIN_BYTES, 11),
        ),
        (
            "exactly-avg.bin",
            pseudo_random(qxet_core::CHUNK_AVG_BYTES, 12),
        ),
        (
            "exactly-max.bin",
            pseudo_random(qxet_core::CHUNK_MAX_BYTES, 13),
        ),
        (
            "max-plus-one.bin",
            pseudo_random(qxet_core::CHUNK_MAX_BYTES + 1, 14),
        ),
        ("all-bytes.bin", (0..400).flat_map(|_| 0u8..=255).collect()),
        ("three-mb.bin", pseudo_random(3 * 1024 * 1024 + 7, 15)),
    ];
    for (name, bytes) in &shapes {
        let src = write_src(&fx, name, bytes);
        let outcome = fx.store.put_file(&src).expect("put");
        assert_eq!(outcome.size_bytes, bytes.len() as u64, "{name}");
        let written = fx.store.get(&outcome.id, &fx.out_dir).expect("get");
        assert_eq!(written.file_name().and_then(|n| n.to_str()), Some(*name));
        assert_eq!(
            &fs::read(&written).expect("read back"),
            bytes,
            "{name} not identical"
        );
        // The prefixed form of the ID works too.
        let written2 = fx
            .store
            .get(&format!("qxet:{}", outcome.id), &fx.out_dir)
            .expect("get prefixed");
        assert_eq!(fs::read(&written2).expect("read back 2"), *bytes);
    }
    let report = fx.store.verify().expect("verify");
    assert!(report.is_clean(), "verify not clean: {report:?}");
    assert_eq!(report.artifacts_checked, shapes.len());
}

#[test]
fn duplicate_put_is_idempotent() {
    let fx = fixture();
    let bytes = pseudo_random(500_000, 21);
    let src = write_src(&fx, "dup.bin", &bytes);
    let first = fx.store.put_file(&src).expect("first put");
    let chunks_after_first = chunk_files(&fx.store).len();
    let second = fx.store.put_file(&src).expect("second put");
    assert_eq!(first.id, second.id);
    assert!(second.already_present);
    assert_eq!(second.new_chunk_count, 0);
    assert_eq!(chunk_files(&fx.store).len(), chunks_after_first);
    assert_eq!(fx.store.list().expect("list").len(), 1);
}

#[test]
fn dedup_shares_chunks_across_artifacts() {
    let fx = fixture();
    let prefix = pseudo_random(300_000, 31);
    let mut longer = prefix.clone();
    longer.extend_from_slice(&pseudo_random(80_000, 32));
    let a = fx
        .store
        .put_file(&write_src(&fx, "a.bin", &prefix))
        .expect("put a");
    let b = fx
        .store
        .put_file(&write_src(&fx, "b.bin", &longer))
        .expect("put b");
    // The longer file reuses nearly all of the shorter one's chunks.
    assert!(
        b.new_chunk_count <= 3,
        "b wrote {} new chunks",
        b.new_chunk_count
    );
    let ma = read_manifest(&fx.store, &a.id);
    let mb = read_manifest(&fx.store, &b.id);
    let union: HashSet<&str> = ma
        .chunks
        .iter()
        .chain(mb.chunks.iter())
        .map(|c| c.hash.as_str())
        .collect();
    // On-disk object count equals the union of referenced chunks:
    // nothing extra, nothing missing, nothing duplicated.
    assert_eq!(chunk_files(&fx.store).len(), union.len());
    // list() reports stored bytes consistent with the manifests.
    let infos = fx.store.list().expect("list");
    let info_b = infos.iter().find(|i| i.id == b.id).expect("b listed");
    assert_eq!(info_b.size_bytes, longer.len() as u64);
    assert!(info_b.stored_bytes <= info_b.size_bytes);
}

#[test]
fn gc_dry_run_then_apply() {
    let fx = fixture();
    let shared = pseudo_random(200_000, 41);
    let mut b_bytes = shared.clone();
    b_bytes.extend_from_slice(&pseudo_random(60_000, 42));
    let a = fx
        .store
        .put_file(&write_src(&fx, "keep.bin", &shared))
        .expect("put a");
    let b = fx
        .store
        .put_file(&write_src(&fx, "gone.bin", &b_bytes))
        .expect("put b");
    // Removing b's manifest orphans the chunks only b referenced.
    fs::remove_file(
        fx.store
            .root()
            .join("manifests")
            .join(format!("{}.json", b.id)),
    )
    .expect("remove manifest");
    let objects_before = chunk_files(&fx.store).len();
    let dry = fx.store.gc(true).expect("gc dry run");
    assert!(dry.orphan_chunk_count >= 1, "expected orphans, got {dry:?}");
    assert_eq!(dry.removed_chunk_count, 0);
    assert_eq!(
        chunk_files(&fx.store).len(),
        objects_before,
        "dry run deleted files"
    );
    let applied = fx.store.gc(false).expect("gc apply");
    assert_eq!(applied.removed_chunk_count, dry.orphan_chunk_count);
    assert_eq!(applied.removed_bytes, dry.orphan_bytes);
    assert_eq!(
        chunk_files(&fx.store).len(),
        objects_before - dry.orphan_chunk_count
    );
    // The surviving artifact is untouched and still verifies.
    let written = fx.store.get(&a.id, &fx.out_dir).expect("get survivor");
    assert_eq!(fs::read(&written).expect("read"), shared);
    assert!(fx.store.verify().expect("verify").is_clean());
    // A second gc finds nothing.
    assert_eq!(fx.store.gc(false).expect("gc again").orphan_chunk_count, 0);
}

#[test]
fn get_to_explicit_path_and_overwrite() {
    let fx = fixture();
    let bytes = pseudo_random(50_000, 51);
    let outcome = fx
        .store
        .put_file(&write_src(&fx, "orig.bin", &bytes))
        .expect("put");
    let dest = fx.out_dir.join("renamed.dat");
    fx.store.get_to(&outcome.id, &dest).expect("get_to");
    assert_eq!(fs::read(&dest).expect("read"), bytes);
    // Overwriting an existing destination works (rename semantics).
    fs::write(&dest, b"stale").expect("seed stale file");
    fx.store
        .get_to(&outcome.id, &dest)
        .expect("get_to overwrite");
    assert_eq!(fs::read(&dest).expect("read"), bytes);
}

#[test]
fn discover_walks_up_from_nested_dir() {
    let fx = fixture();
    let nested = fx._tmp.path().join("a/b/c");
    fs::create_dir_all(&nested).expect("nested");
    let found = Store::discover(&nested).expect("discover");
    assert_eq!(found.root(), fx.store.root());
    let nowhere = tempfile::tempdir().expect("bare tempdir");
    assert!(Store::discover(nowhere.path()).is_err());
}

#[test]
fn concurrent_puts_keep_store_consistent() {
    let fx = fixture();
    let shared_prefix = pseudo_random(200_000, 61);
    let store = &fx.store;
    let src_dir = &fx.src_dir;
    std::thread::scope(|scope| {
        for i in 0..4u64 {
            let prefix = shared_prefix.clone();
            scope.spawn(move || {
                let mut bytes = prefix;
                bytes.extend_from_slice(&pseudo_random(40_000, 100 + i));
                let path = src_dir.join(format!("conc-{i}.bin"));
                fs::write(&path, &bytes).expect("write");
                store.put_file(&path).expect("concurrent put");
            });
        }
    });
    let report = fx.store.verify().expect("verify");
    assert!(
        report.is_clean(),
        "verify after concurrent puts: {report:?}"
    );
    assert_eq!(fx.store.list().expect("list").len(), 4);
    // Every object on disk is referenced (no torn writes).
    assert_eq!(fx.store.gc(true).expect("gc").orphan_chunk_count, 0);
}
