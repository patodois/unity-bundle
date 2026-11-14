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
    let outside = Entry::new("x", 299_990, 100, 0);
    assert!(b.read_entry(&outside).is_err());
    let huge = Entry::new("x", u64::MAX - 5, 100, 0);
    assert!(b.read_entry(&huge).is_err());
    let empty = Entry::new("x", 10, 0, 0);
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
fn padding_before_the_block_info_is_named_in_the_error() {
    // What some Unity 2019.4 bundles are said to have and this crate does not read: the header's
    // 49 bytes, zeros up to offset 64, and only then the block info
    let content = sample_bytes(1000, 2);
    let entries = [EntrySpec {
        path: "f",
        flags: 0,
        offset: 0,
        size: 1000,
    }];
    let (info, body) = bundle_parts(&[(content, Pack::Lz4)], &entries);
    let packed = lz4_compress(&info);
    let with_padding = |padding: usize| {
        let mut w = W::default();
        w.cstr("UnityFS").be_u32(6).cstr("5.x.x").cstr("2019.4.9f1");
        w.be_i64(0)
            .be_u32(packed.len() as u32)
            .be_u32(info.len() as u32)
            .be_u32(0x43);
        assert_eq!(w.0.len(), 49);
        w.bytes(&vec![0u8; padding]).bytes(&packed).bytes(&body);
        w.0
    };
    assert!(Bundle::new(&with_padding(0)[..]).is_ok());

    let err = Bundle::new(&with_padding(15)[..]).unwrap_err();
    assert!(matches!(err, Error::Decompress(_)), "{err:?}");
    assert_eq!(
        err.to_string(),
        "decompression failed: block info: lz4: match offset outside the output \
         (the 15 bytes after the header are zero, up to a multiple of 16: this looks like a \
         bundle whose block info is padded, which this crate does not read)"
    );

    // A block info that fails for another reason gets no such hint: its first byte damaged, and
    // zeros after the header that do not reach a multiple of 16
    let mut damaged = with_padding(0);
    damaged[49] = 0;
    let err = Bundle::new(&damaged[..]).unwrap_err();
    assert!(matches!(err, Error::Decompress(_)), "{err:?}");
    assert!(!err.to_string().contains("padded"), "{err}");
    let err = Bundle::new(&with_padding(7)[..]).unwrap_err();
    assert!(!err.to_string().contains("padded"), "{err}");
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

/// The loop of the README's sample, on a bundle with one texture whose pixels are in the object
/// (an empty `m_StreamData.path`) and one whose pixels are in the `.resS` file.
#[test]
fn the_readme_sample_reads_inline_and_streamed_textures() {
    use serde::Deserialize;
    use unity_bundle::SerializedFile;

    #[derive(Deserialize)]
    #[allow(non_snake_case)]
    struct Texture2D {
        m_Name: String,
        #[serde(rename = "image data")]
        image_data: Vec<u8>,
        m_StreamData: StreamingInfo,
    }
    #[derive(Deserialize)]
    struct StreamingInfo {
        offset: u32,
        size: u32,
        path: String,
    }

    let tree = n(
        "Texture2D",
        "Base",
        vec![
            string("m_Name"),
            N {
                meta: ALIGN,
                ..typeless("image data")
            },
            n(
                "StreamingInfo",
                "m_StreamData",
                vec![
                    prim("unsigned int", "offset"),
                    prim("unsigned int", "size"),
                    string("path"),
                ],
            ),
        ],
    );
    let mut inline = W::default();
    inline.string("inline").u32(3).bytes(&[1, 2, 3]).align4();
    inline.u32(0).u32(0).string("");
    let mut streamed = W::default();
    streamed.string("streamed").u32(0);
    streamed
        .u32(100)
        .u32(50)
        .string("archive:/CAB-t/CAB-t.resS");
    let file = serialized_file(
        21,
        &[TypeSpec {
            class_id: 28,
            tree: Some(tree),
        }],
        &[
            ObjectSpec {
                path_id: 1,
                type_index: 0,
                data: inline.0,
            },
            ObjectSpec {
                path_id: 2,
                type_index: 0,
                data: streamed.0,
            },
        ],
        &[],
        true,
    );
    let res = sample_bytes(400, 9);
    let entries = [
        EntrySpec {
            path: "CAB-t",
            flags: 4,
            offset: 0,
            size: file.len() as u64,
        },
        EntrySpec {
            path: "CAB-t.resS",
            flags: 0,
            offset: file.len() as u64,
            size: res.len() as u64,
        },
    ];
    let bytes = bundle(&[([file, res.clone()].concat(), Pack::Lz4)], &entries);

    let bundle = Bundle::new(&bytes[..]).unwrap();
    let entry = bundle.main_serialized_entry().unwrap();
    let data = bundle.read_entry(entry).unwrap();
    let file = SerializedFile::parse(&data).unwrap();
    let mut seen = Vec::new();
    for info in file.objects().iter().filter(|o| o.class_id.0 == 28) {
        let t: Texture2D = file.read(&data, info).unwrap();
        let pixels = if t.m_StreamData.path.is_empty() {
            t.image_data
        } else {
            let name = t.m_StreamData.path.rsplit('/').next().unwrap();
            let res = bundle.entry(name).expect("no resS");
            bundle
                .read_entry_range(
                    res,
                    t.m_StreamData.offset as u64,
                    t.m_StreamData.size as usize,
                )
                .unwrap()
        };
        seen.push((t.m_Name, pixels));
    }
    assert_eq!(
        seen,
        vec![
            ("inline".to_string(), vec![1, 2, 3]),
            ("streamed".to_string(), res[100..150].to_vec()),
        ]
    );
}

#[test]
fn debug_output_does_not_print_the_data() {
    let (data, _) = sample();
    let b = Bundle::new(&data[..]).unwrap();
    let text = format!("{b:?}");
    assert!(text.len() < 2000, "{} characters", text.len());
    assert!(text.contains("CAB-1.resS"));
    let owned = Bundle::new(data).unwrap();
    assert!(format!("{owned:?}").len() < 2000);
}
