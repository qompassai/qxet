//! Known-answer tests for the FastCDC chunker.
//!
//! The expected values come from the reference implementation's own
//! published tests (`fastcdc` crate, v2020 module, MIT — Nathan
//! Fiedler), which pin the algorithm's behavior on synthetic inputs.
//! If these fail, the port has drifted from the reference algorithm
//! and dedup parity with every other FastCDC store is lost.

use qxet_core::chunker::{Chunker, ChunkerParams};
use qxet_core::gear::{GEAR, GEAR_LS};

fn small_params() -> ChunkerParams {
    // The reference tests' parameters: min 64, avg 256, max 1024.
    ChunkerParams {
        avg_size_bytes: 256,
        max_size_bytes: 1024,
        min_size_bytes: 64,
    }
}

/// Chunk `data` completely by repeated windowed cuts, as the store
/// does, returning the chunk lengths.
fn chunk_lengths(chunker: &Chunker, data: &[u8]) -> Vec<usize> {
    let mut lengths = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let n = chunker.cut(rest);
        assert!(n >= 1 && n <= rest.len(), "cut returned {n}");
        lengths.push(n);
        rest = &rest[n..];
    }
    lengths
}

/// Deterministic pseudo-random bytes (xorshift64*) for property tests.
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

#[test]
fn gear_table_matches_published_values() {
    // Spot values from the reference table (itself generated as the
    // MD5 of a 64-byte block filled with the index byte).
    assert_eq!(GEAR[0], 0x3b5d_3c7d_207e_37dc);
    assert_eq!(GEAR[1], 0x784d_68ba_9112_3086);
    assert_eq!(GEAR[2], 0xcd52_880f_882e_7298);
    assert_eq!(GEAR[255], 0xaabd_2b2a_4515_04e1);
    for (i, v) in GEAR.iter().enumerate() {
        assert_eq!(GEAR_LS[i], v << 1, "GEAR_LS[{i}] is not GEAR[{i}] << 1");
    }
}

#[test]
fn kat_all_zeros_cut_at_maximum() {
    // Reference test_cut_all_zeros: all-zero input always produces
    // maximum-size chunks (no mask ever matches).
    let chunker = Chunker::new(small_params()).expect("params");
    let data = vec![0u8; 10240];
    let lengths = chunk_lengths(&chunker, &data);
    assert_eq!(lengths, vec![1024; 10]);
}

#[test]
fn kat_odd_tail_after_forced_chunk() {
    // Reference test_odd_tail_hash_not_stale (lengths aspect):
    // 1024 + 65 zero bytes cut as [1024, 65].
    let chunker = Chunker::new(small_params()).expect("params");
    let data = vec![0u8; 1024 + 65];
    let lengths = chunk_lengths(&chunker, &data);
    assert_eq!(lengths, vec![1024, 65]);
}

#[test]
fn chunk_bounds_hold_for_random_data_at_default_params() {
    let chunker = Chunker::new(qxet_core::default_chunker_params()).expect("params");
    let data = pseudo_random(3 * 1024 * 1024 + 12345, 42);
    let lengths = chunk_lengths(&chunker, &data);
    assert!(
        lengths.len() > 10,
        "expected many chunks, got {}",
        lengths.len()
    );
    for (i, n) in lengths.iter().enumerate() {
        assert!(*n <= qxet_core::CHUNK_MAX_BYTES, "chunk {i} over max");
        if i + 1 < lengths.len() {
            assert!(*n >= qxet_core::CHUNK_MIN_BYTES, "chunk {i} under min: {n}");
        }
    }
    assert_eq!(lengths.iter().sum::<usize>(), data.len());
}

#[test]
fn single_byte_insertion_shifts_only_local_chunks() {
    // The content-defined property qxet exists for: inserting one
    // byte must not re-chunk the whole file. Chunks covering the
    // first 32 KiB (well before the insertion) are byte-identical,
    // and most chunk hashes survive overall.
    use qxet_core::hash::sha256_hex;
    let chunker = Chunker::new(qxet_core::default_chunker_params()).expect("params");
    let data = pseudo_random(1024 * 1024, 7);
    let hashes_of = |buf: &[u8]| -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = buf;
        while !rest.is_empty() {
            let n = chunker.cut(rest);
            out.push(sha256_hex(&rest[..n]));
            rest = &rest[n..];
        }
        out
    };
    let before = hashes_of(&data);
    let mut modified = data.clone();
    modified.insert(512 * 1024, 0xAB);
    let after = hashes_of(&modified);
    // Prefix stability: chunk hashes agree until the insertion region.
    let shared_prefix = before
        .iter()
        .zip(after.iter())
        .take_while(|(a, b)| a == b)
        .count();
    assert!(
        shared_prefix >= 4,
        "only {shared_prefix} prefix chunks stable"
    );
    // Overall survival: most of the file's chunks are unchanged.
    let before_set: std::collections::HashSet<&String> = before.iter().collect();
    let surviving = after.iter().filter(|h| before_set.contains(h)).count();
    assert!(
        surviving * 2 > after.len(),
        "only {surviving}/{} chunks survived a 1-byte insertion",
        after.len()
    );
}
