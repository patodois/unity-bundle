//! Writes the seed corpora under `fuzz/corpus/<target>/`: small synthetic inputs built with the
//! writers of the tests. No game data is used or needed.
//!
//! `cargo run --manifest-path fuzz/Cargo.toml --bin gen_seeds` (from the repository root).

use std::fs;
use std::path::Path;

use unity_bundle_fuzz::common::*;
use unity_bundle_fuzz::{unwrap_bundle, unwrap_serialized};

fn wanted_tree() -> N {
    n(
        "Wanted",
        "Base",
        vec![
            string("m_Name"),
            prim("int", "m_Value"),
            vector("m_List", prim("int", "")),
            vector(
                "m_Junk",
                n(
                    "P",
                    "",
                    vec![
                        string("s"),
                        N {
                            meta: ALIGN,
                            ..typeless("t")
                        },
                    ],
                ),
            ),
            map("m_Map", string(""), vector("", prim("float", ""))),
            pptr("m_Ref"),
        ],
    )
}

fn wanted_data() -> Vec<u8> {
    let mut w = W::default();
    w.string("name").i32(5);
    w.u32(3).i32(1).i32(2).i32(3);
    w.u32(2)
        .string("a")
        .u32(3)
        .bytes(&[1, 2, 3])
        .align4()
        .string("bc")
        .u32(0);
    w.u32(1).string("k").u32(2).f32(1.0).f32(2.0);
    w.i32(0).i64(7);
    w.0
}

fn prims_tree() -> N {
    n(
        "Prims",
        "Base",
        vec![
            na("bool", "a", vec![]),
            prim("UInt8", "b"),
            prim("UInt16", "d"),
            prim("unsigned int", "f"),
            prim("FileSize", "h"),
            prim("long long", "i"),
            prim("float", "j"),
            prim("double", "k"),
            prim("char", "l"),
        ],
    )
}

fn prims_data() -> Vec<u8> {
    let mut w = W::default();
    w.u8(1).align4();
    w.u8(200).u16(65000).u32(4_000_000_000).u64(1 << 50).i64(-5);
    w.f32(2.5).bytes(&(-0.125f64).to_le_bytes()).u8(b'z');
    w.0
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(name), bytes).unwrap();
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let sfile =
        |v: u32, has_tree: bool, types: Vec<TypeSpec>, objs: Vec<ObjectSpec>, ext: &[&str]| {
            serialized_file(v, &types, &objs, ext, has_tree)
        };
    let mk_types = |tree: N| {
        vec![TypeSpec {
            class_id: 114,
            tree: Some(tree),
        }]
    };
    let obj = |id: i64, data: Vec<u8>| ObjectSpec {
        path_id: id,
        type_index: 0,
        data,
    };

    // Serialized files
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "v21_wanted",
            sfile(
                21,
                true,
                mk_types(wanted_tree()),
                vec![obj(1, wanted_data()), obj(2, wanted_data())],
                &["archive:/CAB-a/CAB-a"],
            ),
        ),
        (
            "v17_wanted",
            sfile(
                17,
                true,
                mk_types(wanted_tree()),
                vec![obj(1, wanted_data())],
                &[],
            ),
        ),
        (
            "v21_prims",
            sfile(
                21,
                true,
                mk_types(prims_tree()),
                vec![obj(5, prims_data())],
                &[],
            ),
        ),
        (
            "v21_no_tree",
            sfile(
                21,
                false,
                vec![TypeSpec {
                    class_id: 1,
                    tree: None,
                }],
                vec![obj(1, vec![1, 2, 3, 4])],
                &[],
            ),
        ),
        ("v21_empty", sfile(21, true, vec![], vec![], &[])),
    ];
    for (name, bytes) in &files {
        write(&root.join("serialized_parse"), name, bytes);
    }

    // Serialized files around a tree blob and an object
    for (name, tree, data) in [
        ("wanted", wanted_tree(), wanted_data()),
        ("prims", prims_tree(), prims_data()),
    ] {
        for (mode, v) in [(0u8, 21u32), (1, 17), (2, 21), (3, 17)] {
            let blob = tree_blob(&tree, v);
            write(
                &root.join("serialized_tree"),
                &format!("{name}_mode{mode}"),
                &unwrap_serialized(mode, &blob, &data),
            );
        }
    }

    // Bundles
    let file = &files[0].1;
    let res = sample_bytes(3000, 3);
    let stream = [file.clone(), res.clone()].concat();
    let entries = [
        EntrySpec {
            path: "CAB-a",
            flags: 4,
            offset: 0,
            size: file.len() as u64,
        },
        EntrySpec {
            path: "CAB-a.resS",
            flags: 0,
            offset: file.len() as u64,
            size: res.len() as u64,
        },
    ];
    let half = stream.len() / 2;
    let layouts: Vec<(&str, Vec<(Vec<u8>, Pack)>)> = vec![
        (
            "lz4_stored",
            vec![
                (stream[..half].to_vec(), Pack::Lz4),
                (stream[half..].to_vec(), Pack::Stored),
            ],
        ),
        ("one_lz4", vec![(stream.clone(), Pack::Lz4)]),
        (
            "literals",
            vec![
                (stream[..half].to_vec(), Pack::Lz4Literals),
                (stream[half..].to_vec(), Pack::Lz4Literals),
            ],
        ),
        ("stored", vec![(stream.clone(), Pack::Stored)]),
    ];
    for (name, blocks) in &layouts {
        write(&root.join("bundle_open"), name, &bundle(blocks, &entries));
        let (info, body) = bundle_parts(blocks, &entries);
        for mode in [0u8, 1] {
            write(
                &root.join("bundle_info"),
                &format!("{name}_mode{mode}"),
                &unwrap_bundle(mode, &info, &body),
            );
        }
    }
    // A bundle with a big LZ4 block of repeated bytes
    let big = vec![vec![7u8; 100_000]];
    let (info, body) = bundle_parts(
        &[(big[0].clone(), Pack::Lz4)],
        &[EntrySpec {
            path: "CAB-b.resS",
            flags: 0,
            offset: 0,
            size: 100_000,
        }],
    );
    write(
        &root.join("bundle_info"),
        "big_run_mode1",
        &unwrap_bundle(1, &info, &body),
    );
    write(&root.join("bundle_open"), "empty_bundle", &bundle(&[], &[]));

    // Trees for de_tree: [Spec bytes][data]. Spec fields are read as ty,name,align, then a
    // children length; the encoding is the fuzzer's own, so these are small hand-written starts.
    for (name, bytes) in [
        ("int", vec![14u8, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]),
        (
            "string",
            vec![20u8, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, b'h', b'i', 0, 0],
        ),
        ("empty", vec![]),
    ] {
        write(&root.join("de_tree"), name, &bytes);
    }
}
