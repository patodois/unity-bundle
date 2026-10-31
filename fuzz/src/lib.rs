//! Shared parts of the fuzz targets: the byte writers of the tests, wrappers that put the fuzzer's
//! bytes inside a valid outer structure (so that mutations reach the inner parser instead of
//! failing a signature check), what a user does with a parsed file, and a check on memory.

#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::de::IgnoredAny;
use serde::Deserialize;
use unity_bundle::bundle::Bundle;
use unity_bundle::serialized::SerializedFile;

/// The writers of the integration tests (bundles, serialized files, type trees, LZ4).
#[path = "../../tests/common/mod.rs"]
pub mod common;

use common::W;

// ---------------------------------------------------------------------------------------------
// Memory

struct Tracking;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
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
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size >= layout.size() {
            grow(new_size - layout.size());
        } else {
            LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: Tracking = Tracking;

/// What the input may justify: LZ4 can stand for 255 bytes per stored byte (`lz4::MAX_RATIO`), and a
/// block table is read once whole, then a block at a time, so twice that plus a fixed part for
/// the structures. Anything above this is an allocation that the input did not pay for.
pub const BYTES_PER_INPUT_BYTE: usize = 600;
pub const FIXED_BYTES: usize = 8 << 20;

/// Runs `f` and panics if it held more live memory at once than `input_len` justifies.
pub fn bounded<T>(input_len: usize, f: impl FnOnce() -> T) -> T {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let r = f();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    let limit = input_len * BYTES_PER_INPUT_BYTE + FIXED_BYTES;
    assert!(
        peak <= limit,
        "{peak} bytes live at once for an input of {input_len} bytes (limit {limit})"
    );
    r
}

// ---------------------------------------------------------------------------------------------
// What a user does with the results

/// A struct that wants some fields and ignores the rest.
#[derive(Deserialize, Debug)]
#[allow(dead_code, non_snake_case)]
pub struct Wanted {
    m_Name: String,
    m_Value: i32,
    #[serde(default)]
    m_List: Vec<i32>,
    #[serde(default)]
    m_Junk: Vec<IgnoredAny>,
}

/// Everything a user does with a bundle, on whatever bytes.
pub fn exercise_bundle(bytes: &[u8]) {
    let Ok(b) = Bundle::new(bytes) else { return };
    let _ = b.header();
    let _ = b.main_serialized_entry();
    for e in b.entries().iter().take(64) {
        let _ = b.entry(&e.path);
        let _ = b.read_entry_range(e, 0, (e.size as usize).min(1 << 16));
        let _ = b.read_entry_range(e, e.size / 2, 100);
        if let Ok(data) = b.read_entry(e) {
            if e.is_serialized() && data.len() < (1 << 22) {
                exercise_serialized(&data);
            }
        }
    }
}

/// Everything a user does with a serialized file.
pub fn exercise_serialized(data: &[u8]) {
    let Ok(f) = SerializedFile::parse(data) else {
        return;
    };
    for info in f.objects().iter().take(256) {
        let _ = f.raw(data, info);
        let _ = f.read::<Wanted>(data, info);
        let _ = f.read::<IgnoredAny>(data, info);
        let _ = f.read::<serde_json::Value>(data, info);
        let _ = f.read_path_id::<IgnoredAny>(data, info.path_id);
        if let Some(t) = f.type_tree(info) {
            let _ = t.dump();
        }
    }
    let _ = f.object(0);
}

// ---------------------------------------------------------------------------------------------
// Wrappers

/// Splits `data` into a chunk whose length the first two bytes give (big-endian, clamped), and the rest.
fn split_len(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let (len, rest) = data.split_first_chunk::<2>()?;
    let n = (u16::from_be_bytes(*len) as usize).min(rest.len());
    Some(rest.split_at(n))
}

/// A UnityFS bundle around fuzzer bytes: `[mode][len u16 BE][block info][blocks]`. Bit 0 of `mode`
/// has the block info stored as is, otherwise it is LZ4-compressed here. The header is valid either way, so the
/// mutations land in the table and in the blocks.
pub fn wrap_bundle(data: &[u8]) -> Option<Vec<u8>> {
    let (&mode, rest) = data.split_first()?;
    let (info, body) = split_len(rest)?;
    let (packed, flags) = if mode & 1 == 1 {
        (info.to_vec(), 0x40)
    } else {
        (common::lz4_compress(info), 0x42)
    };
    let mut f = W::default();
    f.cstr("UnityFS");
    f.be_u32(6);
    f.cstr("5.x.x");
    f.cstr("2019.4.9f1");
    let total = f.0.len() + 8 + 4 + 4 + 4 + packed.len() + body.len();
    f.be_i64(total as i64);
    f.be_u32(packed.len() as u32);
    f.be_u32(info.len() as u32);
    f.be_u32(flags);
    f.bytes(&packed);
    f.bytes(body);
    Some(f.0)
}

/// The inverse of [`wrap_bundle`] for a bundle's pieces, used for the seeds.
pub fn unwrap_bundle(mode: u8, info: &[u8], body: &[u8]) -> Vec<u8> {
    let mut v = vec![mode];
    v.extend_from_slice(&(info.len() as u16).to_be_bytes());
    v.extend_from_slice(info);
    v.extend_from_slice(body);
    v
}

/// A serialized file around fuzzer bytes: `[mode][len u16 BE][type tree blob][object data]`. Bit 0 of `mode` picks format 17
/// (else 21), bit 1 class 114 (a `MonoBehaviour`, which carries a script hash) over class 1. One
/// type holds the blob, one object holds the data, so mutations land in the tree reader and in
/// the deserializer.
pub fn wrap_serialized(data: &[u8]) -> Option<Vec<u8>> {
    let (&mode, rest) = data.split_first()?;
    let (blob, object) = split_len(rest)?;
    let version = if mode & 1 == 1 { 17 } else { 21 };
    let class_id = if mode & 2 == 2 { 114 } else { 1 };
    Some(serialized_with_blob(version, class_id, blob, object))
}

/// One type with the given raw type tree blob and one object with the given data.
pub fn serialized_with_blob(version: u32, class_id: i32, blob: &[u8], object: &[u8]) -> Vec<u8> {
    const HEADER: usize = 20;
    let mut meta = W::default();
    meta.cstr("2019.4.9f1");
    meta.i32(19);
    meta.u8(1); // has type trees
    meta.i32(1);
    meta.i32(class_id);
    meta.u8(0);
    meta.i16(-1);
    if class_id == 114 {
        meta.bytes(&[7; 16]);
    }
    meta.bytes(&[9; 16]);
    meta.bytes(blob);
    if version >= 21 {
        meta.i32(0); // type dependencies
    }
    meta.i32(1); // one object
    while (HEADER + meta.0.len()) % 4 != 0 {
        meta.u8(0);
    }
    meta.i64(1).u32(0).u32(object.len() as u32).i32(0);
    meta.i32(0); // script types
    meta.i32(0); // externals
    if version >= 20 {
        meta.i32(0);
    }
    meta.cstr("");
    let data_offset = (HEADER + meta.0.len() + 15) / 16 * 16;
    let mut f = W::default();
    f.be_u32(meta.0.len() as u32)
        .be_u32((data_offset + object.len()) as u32)
        .be_u32(version)
        .be_u32(data_offset as u32);
    f.u8(0).bytes(&[0; 3]);
    f.bytes(&meta.0);
    while f.0.len() < data_offset {
        f.u8(0);
    }
    f.bytes(object);
    f.0
}

/// Seed form of [`wrap_serialized`].
pub fn unwrap_serialized(mode: u8, blob: &[u8], object: &[u8]) -> Vec<u8> {
    unwrap_bundle(mode, blob, object)
}
