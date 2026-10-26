mod common;

use common::*;
use unity_bundle::bundle::{Bundle, Compression, Entry};
use unity_bundle::Error;

/// Three blocks (stored, LZ4, literal-only LZ4) holding 3 files.
fn sample() -> (Vec<u8>, Vec<u8>) {
    let stream = sample_bytes(300_000, 7);
    let blocks = vec![
        (stream[..100_000].to_vec(), Pack::Stored),
        (stream[100_000..230_000].to_vec(), Pack::Lz4),
        (stream[230_000..].to_vec(), Pack::Lz4Literals),
    ];
    let entries = [
        EntrySpec {
            path: "CAB-1",
            flags: 4,
            offset: 0,
            size: 120_000,
        },
        EntrySpec {
            path: "CAB-1.resS",
            flags: 0,
            offset: 120_000,
            size: 150_000,
        },
        EntrySpec {
            path: "CAB-1.resource",
            flags: 0,
            offset: 270_000,
            size: 30_000,
        },
    ];
    (bundle(&blocks, &entries), stream)
}

#[test]
fn opens_and_lists_what_was_written() {
    let (data, _) = sample();
    let b = Bundle::new(&data[..]).unwrap();
    let h = b.header();
    assert_eq!(h.format_version, 6);
    assert_eq!(h.player_version, "5.x.x");
    assert_eq!(h.unity_version, "2019.4.9f1");
    assert_eq!(h.size, data.len() as u64);
    assert_eq!(h.flags, 0x43);
    assert_eq!(h.block_info_compression(), Compression::Lz4Hc);
    assert_eq!(b.blocks().len(), 3);
    assert_eq!(b.blocks()[0].compression(), Compression::None);
    assert_eq!(b.blocks()[1].compression(), Compression::Lz4Hc);
    assert_eq!(b.blocks()[2].compression(), Compression::Lz4);
    assert_eq!(b.entries().len(), 3);
    let main = b.main_serialized_entry().unwrap();
    assert_eq!(main.path, "CAB-1");
    assert!(main.is_serialized());
    assert!(!b.entry("CAB-1.resS").unwrap().is_serialized());
    assert!(b.entry("nothing").is_none());
}

#[test]
fn the_shared_assets_file_is_not_the_main_one() {
    let blocks = vec![(sample_bytes(100, 1), Pack::Stored)];
    let entries = [
        EntrySpec {
            path: "CAB-2.sharedAssets",
            flags: 4,
            offset: 0,
            size: 50,
        },
        EntrySpec {
            path: "CAB-2",
            flags: 4,
            offset: 50,
            size: 50,
        },
    ];
    let data = bundle(&blocks, &entries);
    let b = Bundle::new(&data[..]).unwrap();
    assert_eq!(b.main_serialized_entry().unwrap().path, "CAB-2");
}

#[test]
fn reads_whole_files() {
    let (data, stream) = sample();
    let b = Bundle::new(&data[..]).unwrap();
    for e in b.entries() {
        let got = b.read_entry(e).unwrap();
        assert_eq!(
            got,
            &stream[e.offset as usize..(e.offset + e.size) as usize],
            "{}",
            e.path
        );
    }
}

#[test]
fn reads_ranges_across_block_boundaries() {
    let (data, stream) = sample();
    let b = Bundle::new(&data[..]).unwrap();
    let e = b.entry("CAB-1.resS").unwrap().clone();
    // The entry starts at 120,000: block 1 covers 100,000..230,000 and block 2 the rest
    for (start, len) in [
        (0u64, 1usize),
        (0, 150_000),
        (109_999, 2),
        (109_999, 40_001),
        (149_999, 1),
        (50_000, 70_000),
        (3, 0),
    ] {
        let got = b.read_entry_range(&e, start, len).unwrap();
        let from = (e.offset + start) as usize;
        assert_eq!(got, &stream[from..from + len], "range {start}+{len}");
    }
    // Ranges are inside the file or an error
    assert!(b.read_entry_range(&e, 149_999, 2).is_err());
    assert!(b.read_entry_range(&e, 150_001, 0).is_err());
    assert!(b.read_entry_range(&e, u64::MAX, 2).is_err());
}

#[test]
fn a_file_outside_the_blocks_is_an_error() {
    let (data, _) = sample();
    let b = Bundle::new(&data[..]).unwrap();
    let outside = Entry {
        path: "x".into(),
        offset: 299_990,
        size: 100,
        flags: 0,
    };
    assert!(b.read_entry(&outside).is_err());
    let huge = Entry {
        path: "x".into(),
        offset: u64::MAX - 5,
        size: 100,
        flags: 0,
    };
    assert!(b.read_entry(&huge).is_err());
    let empty = Entry {
        path: "x".into(),
        offset: 10,
        size: 0,
        flags: 0,
    };
    assert_eq!(b.read_entry(&empty).unwrap(), Vec::<u8>::new());
}

#[test]
fn works_on_owned_and_borrowed_data() {
    let (data, _) = sample();
    let owned = Bundle::new(data.clone()).unwrap();
    let boxed: Bundle<Box<[u8]>> = Bundle::new(data.into_boxed_slice()).unwrap();
    assert_eq!(owned.entries(), boxed.entries());
}

#[test]
fn zero_size_blocks_are_skipped() {
    let blocks = vec![
        (vec![1u8; 10], Pack::Stored),
        (vec![], Pack::Stored),
        (vec![2u8; 10], Pack::Lz4),
    ];
    let entries = [EntrySpec {
        path: "f",
        flags: 0,
        offset: 5,
        size: 10,
    }];
    let data = bundle(&blocks, &entries);
    let b = Bundle::new(&data[..]).unwrap();
    let got = b.read_entry(b.entry("f").unwrap()).unwrap();
    assert_eq!(got, [vec![1u8; 5], vec![2u8; 5]].concat());
}

#[test]
fn refuses_other_formats_with_a_reason() {
    let (data, _) = sample();
    let mut bad = data.clone();
    bad[..8].copy_from_slice(b"UnityWeb");
    assert!(matches!(Bundle::new(&bad[..]), Err(Error::Unsupported(_))));
    let mut bad = data.clone();
    bad[11] = 7; // format version 7
    assert!(matches!(Bundle::new(&bad[..]), Err(Error::Unsupported(_))));
    // flags with the block-info-at-the-end bit
    let mut bad = data.clone();
    let flags_at = 8 + 4 + 6 + 11 + 8 + 4 + 4;
    bad[flags_at + 3] |= 0x80;
    assert!(matches!(Bundle::new(&bad[..]), Err(Error::Unsupported(_))));
    // LZMA blocks
    let mut bad = data;
    bad[flags_at + 3] = 0x41;
    assert!(Bundle::new(&bad[..]).is_err());
}

#[test]
fn every_prefix_of_a_bundle_is_an_error_not_a_panic() {
    let (data, _) = sample();
    for n in 0..data.len().min(400) {
        assert!(Bundle::new(&data[..n]).is_err(), "prefix of {n} bytes");
    }
    // Cut inside the blocks: the table claims more than the file holds
    assert!(matches!(
        Bundle::new(&data[..data.len() - 1]),
        Err(Error::Truncated { .. })
    ));
}

#[test]
fn block_size_claims_are_checked_before_anything_is_allocated() {
    // one LZ4 block of 12 bytes that claims to hold 64 MiB: no LZ4 stream expands 5,000,000:1
    let mut b = bundle(
        &[(vec![0u8; 50], Pack::Lz4)],
        &[EntrySpec {
            path: "f",
            flags: 0,
            offset: 0,
            size: 50,
        }],
    );
    // The block table is inside the compressed block info, so rebuild the bundle by hand
    let info = {
        let mut w = W::default();
        w.bytes(&[0; 16]).be_i32(1);
        w.be_u32(64 << 20).be_u32(12).be_u16(3);
        w.be_i32(1).be_i64(0).be_i64(64 << 20).be_u32(0).cstr("f");
        lz4_compress(&w.0)
    };
    let info_len = info.len();
    let mut w = W::default();
    w.cstr("UnityFS").be_u32(6).cstr("5.x.x").cstr("2019.4.9f1");
    w.be_i64(0)
        .be_u32(info_len as u32)
        .be_u32(16 + 4 + 10 + 4 + 8 + 8 + 4 + 2 + 1)
        .be_u32(0x43);
    w.bytes(&info).bytes(&[0u8; 12]);
    b.clear();
    b.extend_from_slice(&w.0);
    let err = Bundle::new(&b[..]).unwrap_err();
    assert!(
        matches!(err, Error::Invalid(_) | Error::Decompress(_)),
        "{err}"
    );
}
