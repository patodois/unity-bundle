//! LZ4 blocks of many shapes through the public API: every match distance around the 16-byte copy
//! width, runs, matches at the very end of a block, and short blocks.

mod common;

use common::*;
use unity_bundle::Bundle;

fn roundtrip(content: Vec<u8>) {
    let n = content.len() as u64;
    let data = bundle(
        &[(content.clone(), Pack::Lz4)],
        &[EntrySpec {
            path: "f",
            flags: 0,
            offset: 0,
            size: n,
        }],
    );
    let b = Bundle::new(&data[..]).unwrap();
    let got = b.read_entry(b.entry("f").unwrap()).unwrap();
    assert!(
        got == content,
        "round trip of {} bytes failed",
        content.len()
    );
}

/// `period` bytes of noise repeated up to `len` bytes, after `prefix` bytes of other noise
fn periodic(prefix: usize, period: usize, len: usize, seed: u64) -> Vec<u8> {
    let noise = sample_bytes(prefix + period, seed);
    let mut v = noise[..prefix].to_vec();
    let unit = &noise[prefix..];
    while v.len() < len {
        v.extend_from_slice(unit);
    }
    v.truncate(len);
    v
}

#[test]
fn every_distance_around_the_copy_width() {
    for period in (1..=40).chain([63, 64, 65, 255, 256, 257, 1000, 4095, 4096, 65535]) {
        for len in [period + 4, period * 2 + 7, 300, 1000, 5000] {
            roundtrip(periodic(period.max(3), period, len + period, period as u64));
        }
    }
}

#[test]
fn every_short_block() {
    for len in 0..200 {
        roundtrip(sample_bytes(len, len as u64 + 1));
        roundtrip(vec![7; len]);
        roundtrip(periodic(0, 3, len, 9));
    }
}

#[test]
fn matches_that_end_at_the_end_of_the_block() {
    for tail in 0..40 {
        let mut v = periodic(20, 7, 400, 5);
        v.extend(sample_bytes(tail, 77));
        roundtrip(v);
    }
}

#[test]
fn large_blocks() {
    roundtrip(sample_bytes(3_000_000, 5));
    roundtrip(vec![0; 3_000_000]);
    roundtrip(periodic(1000, 77, 2_000_000, 3));
}
