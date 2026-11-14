//! Malformed and hostile input: no panics, and live memory bounded by what the input holds.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use common::*;
use serde::de::IgnoredAny;
use serde::Deserialize;
use unity_bundle::bundle::Bundle;
use unity_bundle::serialized::SerializedFile;

/// Counts the bytes that are allocated and not yet freed, and remembers the most there were.
struct Tracking;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

fn shrink(by: usize) {
    LIVE.fetch_sub(by, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grow(layout.size());
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grow(layout.size());
        System.alloc_zeroed(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        shrink(layout.size());
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size >= layout.size() {
            grow(new_size - layout.size());
        } else {
            shrink(layout.size() - new_size);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: Tracking = Tracking;

/// The tests share the counters, so they run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The most live memory, in bytes, that any step may add while reading these small inputs.
const LIMIT: usize = 8 << 20;

fn measure<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let r = f();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    assert!(
        peak <= LIMIT,
        "{what}: {peak} bytes were live at once (the limit is {LIMIT})"
    );
    r
}

/// What `f` adds to the live memory at its worst, and what it leaves live when it returns.
fn peak_and_kept<T>(f: impl FnOnce() -> T) -> (T, usize, usize) {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let r = f();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    let kept = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    (r, peak, kept)
}

// A struct that wants some fields and ignores the rest, as nocturne-unpack's types do
#[derive(Deserialize, Debug)]
#[allow(dead_code, non_snake_case)]
struct Wanted {
    m_Name: String,
    m_Value: i32,
    #[serde(default)]
    m_List: Vec<i32>,
    #[serde(default)]
    m_Junk: Vec<IgnoredAny>,
}

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
    w.0
}

/// Everything a user does with a bundle, on whatever bytes: must not panic.
fn exercise(bytes: &[u8]) {
    let Ok(b) = Bundle::new(bytes) else { return };
    let _ = b.header();
    for e in b.entries() {
        let _ = b.read_entry_range(e, 0, (e.size as usize).min(1 << 16));
        let _ = b.read_entry_range(e, e.size / 2, 100);
        if e.size < (1 << 24) {
            if let Ok(data) = b.read_entry(e) {
                if e.is_serialized() {
                    exercise_serialized(&data);
                }
            }
        }
    }
}

fn exercise_serialized(data: &[u8]) {
    let Ok(f) = SerializedFile::parse(data) else {
        return;
    };
    for info in f.objects().iter().take(1000) {
        let _ = f.raw(data, info);
        let _ = f.read::<Wanted>(data, info);
        let _ = f.read::<IgnoredAny>(data, info);
        let _ = f.read::<serde_json::Value>(data, info);
        if let Some(t) = f.type_tree(info) {
            let _ = t.dump();
        }
    }
    let _ = f.object(0);
}

fn sample_file() -> Vec<u8> {
    serialized_file(
        21,
        &[TypeSpec {
            class_id: 114,
            tree: Some(wanted_tree()),
        }],
        &[
            ObjectSpec {
                path_id: 1,
                type_index: 0,
                data: wanted_data(),
            },
            ObjectSpec {
                path_id: 2,
                type_index: 0,
                data: wanted_data(),
            },
        ],
        &["archive:/CAB-a/CAB-a"],
        true,
    )
}

fn sample_bundle() -> Vec<u8> {
    let file = sample_file();
    let n = file.len() as u64;
    let res = sample_bytes(5000, 3);
    let stream = [file.clone(), res.clone()].concat();
    let blocks = vec![
        (stream[..stream.len() / 2].to_vec(), Pack::Lz4),
        (stream[stream.len() / 2..].to_vec(), Pack::Stored),
    ];
    let entries = [
        EntrySpec {
            path: "CAB-a",
            flags: 4,
            offset: 0,
            size: n,
        },
        EntrySpec {
            path: "CAB-a.resS",
            flags: 0,
            offset: n,
            size: res.len() as u64,
        },
    ];
    bundle(&blocks, &entries)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut v = base.to_vec();
    for _ in 0..1 + rng.below(4) {
        match rng.below(6) {
            0 => {
                let i = rng.below(v.len());
                v[i] ^= 1 << rng.below(8);
            }
            1 => {
                let i = rng.below(v.len());
                v[i] = rng.next() as u8;
            }
            2 => {
                let n = rng.below(v.len());
                v.truncate(n);
            }
            3 => {
                // overwrite a run with 0xFF or 0x00 (turns sizes and counts huge or zero)
                let i = rng.below(v.len());
                let len = (1 + rng.below(8)).min(v.len() - i);
                let b = if rng.below(2) == 0 { 0xFF } else { 0x00 };
                for x in &mut v[i..i + len] {
                    *x = b;
                }
            }
            4 => {
                let i = rng.below(v.len());
                let j = rng.below(v.len());
                v.swap(i, j);
            }
            _ => {
                // set a 4-byte field to a large value
                if v.len() >= 4 {
                    let i = rng.below(v.len() - 3);
                    let val = [
                        0x7F_FF_FF_FFu32,
                        0x80_00_00_00,
                        0x00_FF_FF_FF,
                        0xFF_FF_FF_F0,
                    ][rng.below(4)];
                    let bytes = if rng.below(2) == 0 {
                        val.to_le_bytes()
                    } else {
                        val.to_be_bytes()
                    };
                    v[i..i + 4].copy_from_slice(&bytes);
                }
            }
        }
        if v.is_empty() {
            v.push(0);
        }
    }
    v
}

#[test]
fn the_samples_are_valid() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = sample_bundle();
    let bundle = Bundle::new(&b[..]).unwrap();
    let data = bundle
        .read_entry(bundle.main_serialized_entry().unwrap())
        .unwrap();
    let f = SerializedFile::parse(&data).unwrap();
    let w: Wanted = f.read_path_id(&data, 2).unwrap();
    assert_eq!(
        (w.m_Name.as_str(), w.m_Value, w.m_List.len(), w.m_Junk.len()),
        ("name", 5, 3, 2)
    );
}

#[test]
fn mutated_bundles_never_panic_and_never_allocate_much() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let base = sample_bundle();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for i in 0..30_000 {
        let m = mutate(&mut rng, &base);
        let r = catch_unwind(AssertUnwindSafe(|| {
            measure("mutated bundle", || exercise(&m))
        }));
        if let Err(e) = r {
            std::fs::write(std::env::temp_dir().join("unity-bundle-crash.bin"), &m).ok();
            std::panic::resume_unwind(e);
        }
        let _ = i;
    }
}

#[test]
fn mutated_block_tables_and_blocks_never_panic_and_never_allocate_much() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let file = sample_file();
    let n = file.len() as u64;
    let res = sample_bytes(5000, 3);
    let stream = [file, res.clone()].concat();
    let blocks = vec![
        (stream[..stream.len() / 3].to_vec(), Pack::Lz4),
        (
            stream[stream.len() / 3..stream.len() / 2].to_vec(),
            Pack::Lz4Literals,
        ),
        (stream[stream.len() / 2..].to_vec(), Pack::Stored),
    ];
    let entries = [
        EntrySpec {
            path: "CAB-a",
            flags: 4,
            offset: 0,
            size: n,
        },
        EntrySpec {
            path: "CAB-a.resS",
            flags: 0,
            offset: n,
            size: res.len() as u64,
        },
    ];
    let (info, body) = bundle_parts(&blocks, &entries);
    assert!(Bundle::new(&assemble_bundle(&info, &body)[..]).is_ok());
    let mut rng = Rng(0xA0761D6478BD642F);
    let mut opened = 0;
    for i in 0..60_000 {
        // mutate the table (then it is compressed again) or the packed blocks
        let bytes = if i % 2 == 0 {
            assemble_bundle(&mutate(&mut rng, &info), &body)
        } else {
            assemble_bundle(&info, &mutate(&mut rng, &body))
        };
        opened += Bundle::new(&bytes[..]).is_ok() as usize;
        let r = catch_unwind(AssertUnwindSafe(|| {
            measure("mutated table", || exercise(&bytes))
        }));
        if let Err(e) = r {
            std::fs::write(std::env::temp_dir().join("unity-bundle-crash.bin"), &bytes).ok();
            std::panic::resume_unwind(e);
        }
    }
    // The mutants must get past the header often enough to test what comes after it
    assert!(opened > 6_000, "only {opened} of 60000 mutants opened");
}

#[test]
fn mutated_serialized_files_never_panic_and_never_allocate_much() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let base = sample_file();
    let mut rng = Rng(0xD1B54A32D192ED03);
    let mut parsed = 0;
    for _ in 0..60_000 {
        let m = mutate(&mut rng, &base);
        parsed += SerializedFile::parse(&m).is_ok() as usize;
        let r = catch_unwind(AssertUnwindSafe(|| {
            measure("mutated file", || exercise_serialized(&m))
        }));
        if let Err(e) = r {
            std::fs::write(std::env::temp_dir().join("unity-bundle-crash.bin"), &m).ok();
            std::panic::resume_unwind(e);
        }
    }
    assert!(parsed > 6_000, "only {parsed} of 60000 mutants parsed");
}

#[test]
fn mutated_objects_never_panic_and_never_allocate_much() {
    use unity_bundle::de::from_slice;
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let file = sample_file();
    let f = SerializedFile::parse(&file).unwrap();
    let tree = f.types[0].type_tree.clone().unwrap();
    let base = wanted_data();
    let mut rng = Rng(0x2545F4914F6CDD1D);
    for _ in 0..100_000 {
        let m = mutate(&mut rng, &base);
        measure("mutated object", || {
            let _ = from_slice::<Wanted>(&m, &tree);
            let _ = from_slice::<IgnoredAny>(&m, &tree);
            let _ = from_slice::<serde_json::Value>(&m, &tree);
        });
    }
}

#[test]
fn every_truncation_and_every_single_byte_change_of_the_samples() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = sample_bundle();
    for n in 0..b.len() {
        measure("truncated bundle", || exercise(&b[..n]));
    }
    let f = sample_file();
    for n in 0..f.len() {
        measure("truncated file", || exercise_serialized(&f[..n]));
    }
    for i in 0..f.len() {
        for v in [0x00u8, 0x01, 0x7F, 0x80, 0xFF] {
            let mut m = f.clone();
            m[i] = v;
            measure("changed file", || exercise_serialized(&m));
        }
    }
    for i in 0..b.len().min(400) {
        for v in [0x00u8, 0x01, 0x7F, 0x80, 0xFF] {
            let mut m = b.clone();
            m[i] = v;
            measure("changed bundle", || exercise(&m));
        }
    }
}

#[test]
fn bombs() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());

    // A type tree whose node count is 2^31 - 1 and whose string table is 2^31 - 1
    let file = sample_file();
    let f = SerializedFile::parse(&file).unwrap();
    // find the blob in the file: after the 114 type's hashes, the node count (i32) follows
    let pos = file.windows(16).position(|w| w == [9; 16]).unwrap() + 16;
    let cases: [(i32, i32); 5] = [
        (i32::MAX, 0),
        (0x00FF_FFFF, 0x7FFF_FFFF),
        (-1, 5),
        (5, -1),
        (1 << 28, 1 << 28),
    ];
    for (nodes, strings) in cases {
        let mut m = file.clone();
        m[pos..pos + 4].copy_from_slice(&nodes.to_le_bytes());
        m[pos + 4..pos + 8].copy_from_slice(&strings.to_le_bytes());
        let r = measure("tree bomb", || SerializedFile::parse(&m));
        assert!(r.is_err(), "{nodes} nodes");
    }
    // Counts of types, objects and externals: the first count is the type count, at a known offset
    let count_at = 20 + "2019.4.9f1\0".len() + 4 + 1;
    for count in [i32::MAX, 1 << 30, 1_000_000, -1] {
        let mut m = file.clone();
        m[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
        let r = measure("type count bomb", || SerializedFile::parse(&m));
        assert!(r.is_err(), "{count} types");
    }
    let _ = f;

    // A block that claims 64 MiB from a handful of bytes, and a file that claims 2^60 bytes
    for (uncompressed, compressed) in [(64u32 << 20, 12u32), (u32::MAX, 20), (1 << 20, 3)] {
        let info = {
            let mut w = W::default();
            w.bytes(&[0; 16]).be_i32(1);
            w.be_u32(uncompressed).be_u32(compressed).be_u16(3);
            w.be_i32(1).be_i64(0).be_i64(1 << 60).be_u32(0).cstr("f");
            lz4_compress(&w.0)
        };
        let mut w = W::default();
        w.cstr("UnityFS").be_u32(6).cstr("5.x.x").cstr("2019.4.9f1");
        w.be_i64(0)
            .be_u32(info.len() as u32)
            .be_u32(60)
            .be_u32(0x43);
        w.bytes(&info).bytes(&vec![0u8; compressed as usize]);
        let r = measure("block bomb", || Bundle::new(&w.0[..]));
        assert!(r.is_err(), "{uncompressed} from {compressed}");
    }

    // A real bundle whose entry claims 2^60 bytes
    let b = sample_bundle();
    let bundle = Bundle::new(&b[..]).unwrap();
    let mut e = bundle.entries()[0].clone();
    e.size = 1 << 60;
    let r = measure("huge entry", || bundle.read_entry(&e));
    assert!(r.is_err());
    e.size = u64::MAX;
    assert!(bundle.read_entry(&e).is_err());

    // Arrays of 2^32 - 1 elements, strings of 2^32 - 1 bytes
    let tree = f_tree();
    let mut data = wanted_data();
    data[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    measure("string bomb", || {
        assert!(unity_bundle::de::from_slice::<Wanted>(&data, &tree).is_err());
    });
    let mut data = wanted_data();
    let list_at = 4 + 4 + 4; // name len, "name", value
    data[list_at..list_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    measure("array bomb", || {
        assert!(unity_bundle::de::from_slice::<Wanted>(&data, &tree).is_err());
    });
}

fn f_tree() -> unity_bundle::TypeTreeNode {
    let file = sample_file();
    let f = SerializedFile::parse(&file).unwrap();
    f.types[0].type_tree.clone().unwrap()
}

/// A serialized file whose only type has the given type tree blob, written as it is.
fn file_with_blob(blob: &[u8]) -> Vec<u8> {
    let small = prim("int", "x");
    let file = serialized_file(
        21,
        &[TypeSpec {
            class_id: 1,
            tree: Some(small.clone()),
        }],
        &[],
        &[],
        true,
    );
    // the blob follows the type's 16-byte hash
    let at = file.windows(16).position(|w| w == [9; 16]).unwrap() + 16;
    let old = tree_blob(&small, 21).len();
    [&file[..at], blob, &file[at + old..]].concat()
}

#[test]
fn nodes_that_share_one_long_string_cost_what_the_tree_costs_not_the_string_times_the_nodes() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // 2,001 nodes (a root and 2,000 children) and a 200,000-byte string buffer, every offset 0:
    // a copy of the string for each type and each name is 800 MB
    let nodes = 2001usize;
    let mut blob = W::default();
    blob.i32(nodes as i32).i32(200_001);
    for i in 0..nodes {
        let level = if i == 0 { 0 } else { 1 };
        blob.u16(1)
            .u8(level)
            .u8(0)
            .u32(0)
            .u32(0)
            .i32(-1)
            .i32(i as i32)
            .i32(0)
            .u64(0);
    }
    blob.bytes(&vec![b'a'; 200_000]).u8(0);
    let file = file_with_blob(&blob.0);
    assert!(file.len() < 300_000);
    let r = measure("shared string bomb", || SerializedFile::parse(&file));
    assert!(
        matches!(
            r,
            Err(unity_bundle::Error::TooLarge {
                what: "type tree string bytes",
                ..
            })
        ),
        "{r:?}"
    );

    // the same tree with short strings is fine
    let mut blob = W::default();
    blob.i32(nodes as i32).i32(8);
    for i in 0..nodes {
        let level = if i == 0 { 0 } else { 1 };
        blob.u16(1)
            .u8(level)
            .u8(0)
            .u32(0)
            .u32(4)
            .i32(-1)
            .i32(i as i32)
            .i32(0)
            .u64(0);
    }
    blob.bytes(b"int\0x\0\0\0");
    let file = file_with_blob(&blob.0);
    let f = measure("shared short string", || SerializedFile::parse(&file)).unwrap();
    assert_eq!(
        f.types[0].type_tree.as_ref().unwrap().children().len(),
        2000
    );
}

fn tree_of(n: &N) -> unity_bundle::TypeTreeNode {
    let file = serialized_file(
        21,
        &[TypeSpec {
            class_id: 1,
            tree: Some(n.clone()),
        }],
        &[],
        &[],
        true,
    );
    SerializedFile::parse(&file).unwrap().types[0]
        .type_tree
        .clone()
        .unwrap()
}

#[test]
fn arrays_of_nothing_inside_arrays_cannot_multiply_the_work() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // outer: 2,000 elements, each an array of "empty" elements. An empty element takes no bytes, so
    // each count only has to fit in the bytes that remain: 8,000 bytes ask for about 8 million
    // elements, which a generic value would hold at 16 bytes each
    let tree = tree_of(&n(
        "S",
        "Base",
        vec![vector(
            "outer",
            n("E", "", vec![vector("inner", n("Nothing", "", vec![]))]),
        )],
    ));
    let outer = 2000u32;
    let mut w = W::default();
    w.u32(outer);
    for k in 0..outer {
        w.u32(4 * (outer - k - 1)); // as many as the bytes that remain allow
    }
    let r = measure("visit bomb", || {
        unity_bundle::de::from_slice::<serde_json::Value>(&w.0, &tree)
    });
    assert!(
        matches!(
            r,
            Err(unity_bundle::Error::TooLarge {
                what: "values to visit",
                ..
            })
        ),
        "{:?}",
        r.map(|_| ())
    );
    let r = measure("visit bomb, skipped", || {
        unity_bundle::de::from_slice::<IgnoredAny>(&w.0, &tree)
    });
    assert!(matches!(r, Err(unity_bundle::Error::TooLarge { .. })));
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // A type tree 250 levels deep, written as a blob, and data for it
    let mut node = prim("int", "leaf");
    for _ in 0..250 {
        node = n("S", "x", vec![node]);
    }
    let blob = tree_blob(&node, 21);
    let mut file = W::default();
    // A file holding only this tree: header, one type, no objects
    let mut meta = W::default();
    meta.cstr("2019.4.9f1")
        .i32(19)
        .u8(1)
        .i32(1)
        .i32(5)
        .u8(0)
        .i16(-1)
        .bytes(&[9; 16])
        .bytes(&blob)
        .i32(0);
    meta.i32(0).i32(0).i32(0).i32(0).cstr("");
    let data_offset = (20 + meta.0.len() + 15) / 16 * 16;
    file.be_u32(meta.0.len() as u32)
        .be_u32(data_offset as u32)
        .be_u32(21)
        .be_u32(data_offset as u32);
    file.u8(0).bytes(&[0; 3]).bytes(&meta.0);
    while file.0.len() < data_offset {
        file.u8(0);
    }
    let f = SerializedFile::parse(&file.0).unwrap();
    let tree = f.types[0].type_tree.as_ref().unwrap();
    let value = 5i32.to_le_bytes();
    let v: serde_json::Value = unity_bundle::de::from_slice(&value, tree).unwrap();
    let mut depth = 0;
    let mut cur = &v;
    while cur.is_object() {
        depth += 1;
        cur = cur.as_object().unwrap().values().next().unwrap();
    }
    assert_eq!(depth, 250);
    assert_eq!(cur, &serde_json::json!(5));
    drop(v);
    // dropping and cloning the deep tree is fine too
    let c = tree.clone();
    assert_eq!(c, *tree);
}

/// The start of a format `version` file with one type that has a type tree: the header and the
/// metadata up to the type's tree blob, and what follows the blob.
fn file_around_blob(blob: &[u8], version: u32) -> Vec<u8> {
    let mut w = W::default();
    w.be_u32(0).be_u32(0).be_u32(version).be_u32(0);
    w.u8(0).bytes(&[0; 3]);
    w.cstr("2019.4.9f1").i32(19).u8(1);
    w.i32(1).i32(1).u8(0).i16(-1).bytes(&[9; 16]).bytes(blob);
    if version >= 21 {
        w.i32(0); // type dependencies
    }
    w.i32(0).i32(0).i32(0); // objects, script types, externals
    if version >= 20 {
        w.i32(0); // reference types
    }
    w.cstr("");
    w.0
}

/// The README states what each table costs as a multiple of the bytes it is read from ("Hostile
/// input"). These are the inputs that cost the most for their size, and the multiples are checked.
#[test]
fn the_tables_cost_the_multiples_the_readme_states() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    const SLACK: usize = 4096;

    // Opening a bundle: a block info of 1 MiB of empty stored blocks (4 KB once compressed). The
    // block table is 12 bytes and the block offsets 16 for each 10-byte record, and the block
    // info is let go before the offsets are built: 2.8 times the block info, not 3.8
    let mut info = W::default();
    let blocks = ((1 << 20) - 24) / 10;
    info.bytes(&[0; 16]).be_i32(blocks as i32);
    info.bytes(&vec![0u8; blocks * 10]).be_i32(0);
    let data = assemble_bundle(&info.0, &[]);
    assert!(data.len() < 5000, "{} bytes", data.len());
    let (b, peak, kept) = peak_and_kept(|| Bundle::new(&data[..]));
    assert_eq!(b.unwrap().blocks().len(), blocks);
    let bound = info.0.len() * 28 / 10 + SLACK;
    assert!(peak <= bound, "opening: {peak} bytes at once, over {bound}");
    assert!(kept <= bound, "opening: {kept} bytes kept, over {bound}");

    // A type table without type trees: 240 bytes for each 23-byte record
    let mut w = W::default();
    w.be_u32(0).be_u32(0).be_u32(21).be_u32(0);
    w.u8(0).bytes(&[0; 3]);
    w.cstr("2019.4.9f1").i32(19).u8(0);
    w.i32(20_000).bytes(&vec![0u8; 20_000 * 23]);
    w.i32(0).i32(0).i32(0).i32(0).cstr("");
    let (f, peak, _) = peak_and_kept(|| SerializedFile::parse(&w.0));
    assert_eq!(f.unwrap().types.len(), 20_000);
    let bound = w.0.len() * 240 / 23 + SLACK;
    assert!(peak <= bound, "type table: {peak} bytes, over {bound}");

    // An object table: 32 bytes and at most 39 of index for each 20-byte record, under 4 times
    let mut w = W::default();
    w.be_u32(0).be_u32(0).be_u32(21).be_u32(0);
    w.u8(0).bytes(&[0; 3]);
    w.cstr("2019.4.9f1").i32(19).u8(0);
    w.i32(1).bytes(&[0u8; 23]).i32(14_400);
    while w.0.len() % 4 != 0 {
        w.u8(0);
    }
    w.bytes(&vec![0u8; 14_400 * 20]);
    w.i32(0).i32(0).i32(0).cstr("");
    let (f, peak, _) = peak_and_kept(|| SerializedFile::parse(&w.0));
    assert_eq!(f.unwrap().objects().len(), 14_400);
    let bound = w.0.len() * 4;
    assert!(peak <= bound, "object table: {peak} bytes, over {bound}");

    // Type trees. The tree that costs the most for its records is one where every node has one
    // child (a root with 100 chains, 200 levels deep): a list of children that grew by pushing
    // kept room for four nodes to hold one, 416 bytes for each node. Kept: 104 bytes for each
    // node and its strings (at most 64 for each node and the tree's string buffer). While it is
    // built: 112 bytes more for each node.
    for version in [17u32, 21] {
        let nodes = 1 + 100 * 200;
        let mut blob = W::default();
        blob.i32(nodes as i32).i32(1);
        let mut index = 0;
        let mut record = |blob: &mut W, level: u8| {
            blob.u16(1).u8(level).u8(0).u32(0).u32(0);
            blob.i32(-1).i32(index).i32(0);
            if version >= 19 {
                blob.u64(0);
            }
            index += 1;
        };
        record(&mut blob, 0);
        for _ in 0..100 {
            for level in 1..=200u8 {
                record(&mut blob, level);
            }
        }
        blob.u8(0); // the string buffer: one empty string
        let file = file_around_blob(&blob.0, version);
        let (f, peak, kept) = peak_and_kept(|| SerializedFile::parse(&file));
        let f = f.unwrap();
        let bound = nodes * 104 + SLACK;
        assert!(
            kept <= bound,
            "format {version} tree: {kept} bytes kept, over {bound}"
        );
        let bound = nodes * 216 + SLACK;
        assert!(
            peak <= bound,
            "format {version} tree: {peak} bytes at once, over {bound}"
        );
        drop(f);

        // With strings as long as the budget lets every node have: 64 bytes more for each node
        let nodes = 20_001;
        let mut blob = W::default();
        blob.i32(nodes as i32).i32(33);
        for i in 0..nodes {
            blob.u16(1).u8((i != 0) as u8).u8(0).u32(0).u32(0);
            blob.i32(-1).i32(i as i32).i32(0);
            if version >= 19 {
                blob.u64(0);
            }
        }
        blob.bytes(&[b'a'; 32]).u8(0);
        let file = file_around_blob(&blob.0, version);
        let (f, peak, kept) = peak_and_kept(|| SerializedFile::parse(&file));
        let f = f.unwrap();
        let bound = nodes * (104 + 64) + SLACK;
        assert!(
            kept <= bound,
            "format {version} tree with strings: {kept} bytes kept, over {bound}"
        );
        let bound = nodes * (216 + 64) + SLACK;
        assert!(
            peak <= bound,
            "format {version} tree with strings: {peak} bytes at once, over {bound}"
        );
        drop(f);
    }
}
