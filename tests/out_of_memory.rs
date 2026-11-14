//! Memory that cannot be had: every table and buffer whose size comes from a count or a length in
//! the input is asked for in a way that can fail, and the failure is `Error::OutOfMemory`, not the
//! end of the process.
//!
//! The allocator of this test binary refuses, on the thread that asks it to, every allocation
//! above a size. Each input here is small, and its count or length is one the input can back (it
//! passes the checks against the bytes that remain), so the only thing that stops the read is the
//! allocation. With an infallible allocation (`Vec::with_capacity`, `vec![0; n]`, `to_vec`) the
//! process aborts and the whole binary fails.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::*;
use serde::Deserialize;
use unity_bundle::bundle::Bundle;
use unity_bundle::serialized::SerializedFile;
use unity_bundle::Error;

struct Refusing;

thread_local! {
    /// The largest allocation this thread is given
    static LARGEST: Cell<usize> = const { Cell::new(usize::MAX) };
}

fn refused(size: usize) -> bool {
    LARGEST.try_with(|l| size > l.get()).unwrap_or(false)
}

unsafe impl GlobalAlloc for Refusing {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if refused(layout.size()) {
            return std::ptr::null_mut();
        }
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if refused(layout.size()) {
            return std::ptr::null_mut();
        }
        System.alloc_zeroed(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if refused(new_size) {
            return std::ptr::null_mut();
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: Refusing = Refusing;

/// Runs `f` on this thread with no allocation above `largest` bytes.
fn with_no_allocation_above<T>(largest: usize, f: impl FnOnce() -> T) -> T {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            LARGEST.with(|l| l.set(usize::MAX));
        }
    }
    let _restore = Restore;
    LARGEST.with(|l| l.set(largest));
    f()
}

/// The error must be the allocation of `what`, and of more than was allowed.
fn assert_out_of_memory<T>(r: Result<T, Error>, what: &str, largest: usize) {
    match r {
        Err(Error::OutOfMemory { what: w, bytes }) => {
            assert_eq!(w, what);
            // (the index is a hash table: the error states its entries, the table is larger)
            assert!(
                bytes > largest as u64 || what == "object index",
                "{what}: {bytes} bytes"
            );
        }
        Err(e) => panic!("{what}: another error: {e}"),
        Ok(_) => panic!("{what}: no error"),
    }
}

/// The 20-byte header and the start of the metadata of a format 21 file, up to the type count.
fn file_start(has_type_tree: bool) -> W {
    let mut w = W::default();
    // metadata size, file size, format version, data offset (0: the objects are not read here)
    w.be_u32(0).be_u32(0).be_u32(21).be_u32(0);
    w.u8(0).bytes(&[0; 3]);
    w.cstr("2019.4.9f1").i32(19).u8(has_type_tree as u8);
    w
}

#[test]
fn a_type_table_that_cannot_be_allocated() {
    // 2,000 type records of 23 zero bytes (class 0, no tree): 46 KB that become 480 KB of table
    let mut w = file_start(false);
    w.i32(2000).bytes(&vec![0u8; 2000 * 23]);
    w.i32(0).i32(0).i32(0).i32(0).cstr("");
    assert!(w.0.len() < 50_000);
    assert_eq!(SerializedFile::parse(&w.0).unwrap().types.len(), 2000);
    let largest = 256 << 10;
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "type table", largest);
}

#[test]
fn an_object_table_and_its_index_that_cannot_be_allocated() {
    // One type and 7,200 object records of 20 zero bytes: 144 KB that become a 230 KB table and
    // a 278 KB index by path id
    let mut w = file_start(false);
    w.i32(1).bytes(&[0u8; 23]);
    w.i32(7200);
    while w.0.len() % 4 != 0 {
        w.u8(0);
    }
    w.bytes(&vec![0u8; 7200 * 20]);
    w.i32(0).i32(0).i32(0).cstr("");
    assert_eq!(SerializedFile::parse(&w.0).unwrap().objects().len(), 7200);
    // the table itself
    let largest = 200_000;
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "object table", largest);
    // the table fits and the index does not
    let largest = 250_000;
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "object index", largest);
}

#[test]
fn script_types_and_externals_that_cannot_be_allocated() {
    // 20,000 script types of 12 bytes: 240 KB that become 320 KB
    let mut w = file_start(false);
    w.i32(0).i32(0);
    w.i32(20_000);
    while w.0.len() % 4 != 0 {
        w.u8(0);
    }
    w.bytes(&vec![0u8; 20_000 * 12]);
    w.i32(0).i32(0).cstr("");
    assert_eq!(
        SerializedFile::parse(&w.0).unwrap().script_types.len(),
        20_000
    );
    let largest = 256 << 10;
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "script type table", largest);

    // 5,000 externals of 22 zero bytes: 110 KB that become 360 KB
    let mut w = file_start(false);
    w.i32(0).i32(0).i32(0);
    w.i32(5000).bytes(&vec![0u8; 5000 * 22]);
    w.i32(0).cstr("");
    assert_eq!(SerializedFile::parse(&w.0).unwrap().externals.len(), 5000);
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "external table", largest);

    // 5,000 reference types of 23 zero bytes: 115 KB that become 1.2 MB
    let mut w = file_start(false);
    w.i32(0).i32(0).i32(0).i32(0);
    w.i32(5000).bytes(&vec![0u8; 5000 * 23]);
    w.cstr("");
    // (script type index 0 on a reference type means a script hash follows: fewer records fit,
    // which is an error of its own, so only the allocation is tried here)
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "reference type table", largest);
}

#[test]
fn a_type_tree_and_its_dependencies_that_cannot_be_allocated() {
    // A tree of 5,000 node records (160 KB): the count is checked against the bytes, and the
    // working table for it is more than twice the records
    let mut w = file_start(true);
    w.i32(1).bytes(&[0u8; 23]);
    w.i32(5000).i32(0).bytes(&vec![0u8; 5000 * 32]);
    let largest = 256 << 10;
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "type tree nodes", largest);

    // A small tree followed by 100,000 type dependencies (400 KB)
    let mut w = file_start(true);
    w.i32(1).bytes(&[0u8; 23]);
    w.bytes(&tree_blob(&prim("int", "x"), 21));
    w.i32(100_000).bytes(&vec![0u8; 100_000 * 4]);
    w.i32(0).i32(0).i32(0).i32(0).cstr("");
    assert_eq!(
        SerializedFile::parse(&w.0).unwrap().types[0]
            .type_dependencies
            .len(),
        100_000
    );
    let r = with_no_allocation_above(largest, || SerializedFile::parse(&w.0));
    assert_out_of_memory(r, "type dependencies", largest);
}

/// A bundle around a block info given as it is stored (`flags` says how).
fn bundle_with_info(stored_info: &[u8], info_len: usize, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut w = W::default();
    w.cstr("UnityFS").be_u32(6).cstr("5.x.x").cstr("2019.4.9f1");
    w.be_i64(0)
        .be_u32(stored_info.len() as u32)
        .be_u32(info_len as u32)
        .be_u32(flags);
    w.bytes(stored_info).bytes(body);
    w.0
}

#[test]
fn a_block_table_that_cannot_be_allocated() {
    // 30,000 block records of 10 zero bytes (empty stored blocks) and no files: 300 KB of block
    // info, kept as it is, that become a 360 KB block table and 480 KB of block offsets
    let mut info = W::default();
    info.bytes(&[0; 16]).be_i32(30_000);
    info.bytes(&vec![0u8; 30_000 * 10]).be_i32(0);
    let stored = bundle_with_info(&info.0, info.0.len(), 0x40, &[]);
    assert_eq!(Bundle::new(&stored[..]).unwrap().blocks().len(), 30_000);
    let largest = 330_000;
    let r = with_no_allocation_above(largest, || Bundle::new(&stored[..]));
    assert_out_of_memory(r, "block table", largest);
    let largest = 420_000;
    let r = with_no_allocation_above(largest, || Bundle::new(&stored[..]));
    assert_out_of_memory(r, "block offsets", largest);

    // The copy of a stored block info, and the output of a compressed one (1.2 KB that
    // decompress to 300 KB)
    let largest = 256 << 10;
    let r = with_no_allocation_above(largest, || Bundle::new(&stored[..]));
    assert_out_of_memory(r, "stored block", largest);
    let packed = lz4_compress(&info.0);
    assert!(packed.len() < 1500);
    let packed = bundle_with_info(&packed, info.0.len(), 0x43, &[]);
    assert_eq!(Bundle::new(&packed[..]).unwrap().blocks().len(), 30_000);
    let r = with_no_allocation_above(largest, || Bundle::new(&packed[..]));
    assert_out_of_memory(r, "decompressed block", largest);
}

#[test]
fn a_directory_that_cannot_be_allocated() {
    // 10,000 files of 21 zero bytes (offset 0, size 0, an empty name): 210 KB that become 480 KB
    let mut info = W::default();
    info.bytes(&[0; 16]).be_i32(0).be_i32(10_000);
    info.bytes(&vec![0u8; 10_000 * 21]);
    let data = bundle_with_info(&info.0, info.0.len(), 0x40, &[]);
    assert_eq!(Bundle::new(&data[..]).unwrap().entries().len(), 10_000);
    let largest = 256 << 10;
    let r = with_no_allocation_above(largest, || Bundle::new(&data[..]));
    assert_out_of_memory(r, "file table", largest);
}

#[test]
fn a_file_of_a_bundle_that_cannot_be_allocated() {
    // Ten stored blocks of 100 KB and one file over all of them
    let content = sample_bytes(1_000_000, 11);
    let blocks: Vec<(Vec<u8>, Pack)> = content
        .chunks(100_000)
        .map(|c| (c.to_vec(), Pack::Stored))
        .collect();
    let data = bundle(
        &blocks,
        &[EntrySpec {
            path: "f",
            flags: 0,
            offset: 0,
            size: 1_000_000,
        }],
    );
    let b = Bundle::new(&data[..]).unwrap();
    let entry = b.entry("f").unwrap();
    assert_eq!(b.read_entry(entry).unwrap(), content);
    let largest = 300_000;
    let r = with_no_allocation_above(largest, || b.read_entry(entry));
    assert_out_of_memory(r, "file", largest);
    // a range that fits is still read
    let part = with_no_allocation_above(largest, || b.read_entry_range(entry, 150_000, 200_000));
    assert_eq!(part.unwrap(), &content[150_000..350_000]);
}

#[test]
fn a_string_of_an_object_that_cannot_be_allocated() {
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct S {
        s: String,
    }
    let file = serialized_file(
        21,
        &[TypeSpec {
            class_id: 1,
            tree: Some(n("S", "Base", vec![string("s")])),
        }],
        &[],
        &[],
        true,
    );
    let f = SerializedFile::parse(&file).unwrap();
    let tree = f.types[0].type_tree.as_ref().unwrap();
    let mut w = W::default();
    w.string(&"a".repeat(100_000));
    assert_eq!(
        unity_bundle::de::from_slice::<S>(&w.0, tree)
            .unwrap()
            .s
            .len(),
        100_000
    );
    let largest = 64 << 10;
    let r = with_no_allocation_above(largest, || unity_bundle::de::from_slice::<S>(&w.0, tree));
    assert_out_of_memory(r, "string", largest);
}
