//! Compares unity-bundle with the rabex fork (the oracle) on every bundle of an installed game.
//!
//! For each file: the container (header, block table, directory), the bytes of the decompressed
//! stream, ranged reads, the serialized file (tables, type trees), a digest of every object read
//! generically through both deserializers, and the typed values of the classes nocturne-unpack
//! reads. Prints counts and every difference with a short description.
//!
//! usage: oracle GAME_DIR [--threads N] [--budget-mb N] [--only SUBSTRING] [--no-values]

mod unity_types;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Write as _};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use memmap2::Mmap;
use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use unity_types::*;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct StreamingInfo {
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub path: String,
}

// ---------------------------------------------------------------------------------------------
// Allocation tracking (for the hostile-input pass)

struct Tracking;

static TRACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe impl std::alloc::GlobalAlloc for Tracking {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            LARGEST.fetch_max(l.size(), Ordering::Relaxed);
        }
        std::alloc::System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            LARGEST.fetch_max(l.size(), Ordering::Relaxed);
        }
        std::alloc::System.alloc_zeroed(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        std::alloc::System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            LARGEST.fetch_max(n, Ordering::Relaxed);
        }
        std::alloc::System.realloc(p, l, n)
    }
}

#[global_allocator]
static ALLOC: Tracking = Tracking;

// ---------------------------------------------------------------------------------------------
// Report

#[derive(Default)]
struct Report {
    counts: BTreeMap<String, u64>,
    diff_counts: BTreeMap<String, u64>,
    diff_samples: BTreeMap<String, Vec<String>>,
    notes: BTreeMap<String, BTreeSet<String>>,
}

static REPORT: Mutex<Option<Report>> = Mutex::new(None);

fn with_report<T>(f: impl FnOnce(&mut Report) -> T) -> T {
    let mut g = REPORT.lock().unwrap();
    f(g.get_or_insert_with(Report::default))
}

fn count(key: &str, n: u64) {
    with_report(|r| *r.counts.entry(key.to_string()).or_default() += n);
}

fn diff(category: &str, msg: String) {
    with_report(|r| {
        *r.diff_counts.entry(category.to_string()).or_default() += 1;
        let s = r.diff_samples.entry(category.to_string()).or_default();
        if s.len() < 30 {
            s.push(msg);
        }
    });
}

fn note(category: &str, msg: String) {
    with_report(|r| {
        let s = r.notes.entry(category.to_string()).or_default();
        if s.len() < 40 {
            s.insert(msg);
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Memory budget

struct Budget {
    used: Mutex<usize>,
    cv: Condvar,
    cap: usize,
}

impl Budget {
    fn acquire(&self, n: usize) -> BudgetGuard<'_> {
        let mut u = self.used.lock().unwrap();
        while *u > 0 && *u + n > self.cap {
            u = self.cv.wait(u).unwrap();
        }
        *u += n;
        BudgetGuard { b: self, n }
    }
}

struct BudgetGuard<'a> {
    b: &'a Budget,
    n: usize,
}

impl Drop for BudgetGuard<'_> {
    fn drop(&mut self) {
        *self.b.used.lock().unwrap() -= self.n;
        self.b.cv.notify_all();
    }
}

// ---------------------------------------------------------------------------------------------
// Hashing

#[derive(Clone, Copy)]
struct Fnv(u64, u64);

impl Fnv {
    const fn new() -> Fnv {
        Fnv(0xcbf29ce484222325, 0)
    }
    #[inline]
    fn byte(&mut self, b: u8) {
        self.0 = (self.0 ^ b as u64).wrapping_mul(0x100000001b3);
        self.1 += 1;
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.byte(x);
        }
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
}

impl std::fmt::Write for Fnv {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.bytes(s.as_bytes());
        Ok(())
    }
}

thread_local! {
    static HASH: std::cell::Cell<Fnv> = const { std::cell::Cell::new(Fnv::new()) };
}

fn feed(f: impl FnOnce(&mut Fnv)) {
    HASH.with(|h| {
        let mut x = h.get();
        f(&mut x);
        h.set(x);
    });
}

/// Hashes every event the deserializer produces through `deserialize_any`.
struct Dig;

impl<'de> Deserialize<'de> for Dig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Dig, D::Error> {
        d.deserialize_any(DigVisitor)?;
        Ok(Dig)
    }
}

struct DigVisitor;

macro_rules! dig_num {
    ($name:ident, $t:ty, $tag:expr) => {
        fn $name<E>(self, v: $t) -> Result<Dig, E> {
            feed(|h| {
                h.byte($tag);
                h.bytes(&v.to_le_bytes());
            });
            Ok(Dig)
        }
    };
}

impl<'de> Visitor<'de> for DigVisitor {
    type Value = Dig;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("anything")
    }
    fn visit_bool<E>(self, v: bool) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b'b');
            h.byte(v as u8);
        });
        Ok(Dig)
    }
    dig_num!(visit_i8, i8, b'1');
    dig_num!(visit_i16, i16, b'2');
    dig_num!(visit_i32, i32, b'4');
    dig_num!(visit_i64, i64, b'8');
    dig_num!(visit_u8, u8, b'a');
    dig_num!(visit_u16, u16, b'c');
    dig_num!(visit_u32, u32, b'e');
    dig_num!(visit_u64, u64, b'g');
    fn visit_f32<E>(self, v: f32) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b'f');
            h.bytes(&v.to_bits().to_le_bytes());
        });
        Ok(Dig)
    }
    fn visit_f64<E>(self, v: f64) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b'd');
            h.bytes(&v.to_bits().to_le_bytes());
        });
        Ok(Dig)
    }
    fn visit_char<E>(self, v: char) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b'C');
            h.u64(v as u64);
        });
        Ok(Dig)
    }
    fn visit_str<E>(self, v: &str) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b's');
            h.u64(v.len() as u64);
            h.bytes(v.as_bytes());
        });
        Ok(Dig)
    }
    fn visit_string<E>(self, v: String) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b'S');
            h.u64(v.len() as u64);
            h.bytes(v.as_bytes());
        });
        Ok(Dig)
    }
    fn visit_bytes<E>(self, v: &[u8]) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b'y');
            h.u64(v.len() as u64);
            h.bytes(v);
        });
        Ok(Dig)
    }
    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Dig, E> {
        feed(|h| {
            h.byte(b'Y');
            h.u64(v.len() as u64);
            h.bytes(&v);
        });
        Ok(Dig)
    }
    fn visit_unit<E>(self) -> Result<Dig, E> {
        feed(|h| h.byte(b'u'));
        Ok(Dig)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Dig, A::Error> {
        feed(|h| h.byte(b'['));
        let mut n = 0u64;
        while seq.next_element::<Dig>()?.is_some() {
            n += 1;
        }
        feed(|h| {
            h.byte(b']');
            h.u64(n);
        });
        Ok(Dig)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Dig, A::Error> {
        feed(|h| h.byte(b'{'));
        let mut n = 0u64;
        while map.next_key::<Dig>()?.is_some() {
            map.next_value::<Dig>()?;
            n += 1;
        }
        feed(|h| {
            h.byte(b'}');
            h.u64(n);
        });
        Ok(Dig)
    }
}

// ---------------------------------------------------------------------------------------------
// Result of a read through one library: a digest or an error message

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Ok(u64, u64),
    Err(String),
}

fn digest_of<E: std::fmt::Display>(r: Result<Dig, E>, start: Fnv) -> Outcome {
    match r {
        Ok(_) => {
            let h = HASH.with(|h| h.get());
            let _ = start;
            Outcome::Ok(h.0, h.1)
        }
        Err(e) => Outcome::Err(e.to_string()),
    }
}

fn reset_hash() -> Fnv {
    HASH.with(|h| h.set(Fnv::new()));
    Fnv::new()
}

fn debug_hash<T: Debug>(v: &T) -> (u64, u64) {
    let mut h = Fnv::new();
    let _ = write!(h, "{v:?}");
    (h.0, h.1)
}

fn first_difference(a: &str, b: &str) -> String {
    let i = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let lo = i.saturating_sub(40);
    let cut = |s: &str| {
        let hi = (i + 60).min(s.len());
        let mut lo = lo.min(s.len());
        let mut hi = hi;
        while !s.is_char_boundary(lo) {
            lo -= 1;
        }
        while !s.is_char_boundary(hi) {
            hi += 1;
        }
        s[lo..hi].to_string()
    };
    format!("at {i}: mine …{}… theirs …{}…", cut(a), cut(b))
}

// ---------------------------------------------------------------------------------------------
// Container

struct Options {
    values: bool,
    only: Option<String>,
}

fn rabex_config() -> rabex::files::bundlefile::ExtractionConfig {
    rabex::files::bundlefile::ExtractionConfig::default().assume_recent_unity()
}

const CHUNK: usize = 8 << 20;

fn process_bundle(path: &Path, label: &str, opts: &Options, budget: &Budget) {
    count("files_total", 1);
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            diff("io", format!("{label}: {e}"));
            return;
        }
    };
    let map = unsafe { Mmap::map(&file) }.expect("mmap");
    let bytes: &[u8] = &map[..];
    count("bytes_on_disk", bytes.len() as u64);

    let mine = unity_bundle::Bundle::new(bytes);
    let theirs = rabex::files::bundlefile::BundleFileReader::from_reader(
        Cursor::new(bytes),
        &rabex_config(),
    );

    let (mine, theirs) = match (mine, theirs) {
        (Ok(m), Ok(t)) => (m, t),
        (Err(m), Err(t)) => {
            count("bundle_open_error_in_both", 1);
            note(
                "bundle_open_error_in_both",
                format!("{label}: mine `{m}` / theirs `{t}`"),
            );
            return;
        }
        (Ok(_), Err(t)) => {
            diff(
                "open: mine opens, theirs fails",
                format!("{label}: theirs `{t}`"),
            );
            return;
        }
        (Err(m), Ok(_)) => {
            diff(
                "open: theirs opens, mine fails",
                format!("{label}: mine `{m}`"),
            );
            return;
        }
    };
    count("bundles_opened_by_both", 1);

    // Header
    let mh = mine.header();
    let th = theirs.header();
    if mh.player_version != th.unity_version {
        diff(
            "header",
            format!(
                "{label}: player version {:?} vs {:?}",
                mh.player_version, th.unity_version
            ),
        );
    }
    let their_rev = th
        .unity_revision
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "0.0.0".into());
    if mh.unity_version != their_rev {
        diff(
            "header",
            format!(
                "{label}: unity version {:?} vs {:?}",
                mh.unity_version, their_rev
            ),
        );
    }
    if mh.size as u32 != th.size {
        diff(
            "header",
            format!("{label}: size {} vs {}", mh.size, th.size),
        );
    }
    if mh.format_version != th.version {
        diff(
            "header",
            format!("{label}: version {} vs {}", mh.format_version, th.version),
        );
    }
    note(
        "header flags",
        format!(
            "flags {:#x}, version {}, player {}, unity {}",
            mh.flags, mh.format_version, mh.player_version, mh.unity_version
        ),
    );
    if mh.size != bytes.len() as u64 {
        note(
            "header size differs from file size",
            format!("{label}: header {} file {}", mh.size, bytes.len()),
        );
        count("header_size_ne_file_size", 1);
    }

    // Blocks
    let mb = mine.blocks();
    let tb = theirs.blocks();
    count("blocks", mb.len() as u64);
    if mb.len() != tb.len() {
        diff(
            "blocks",
            format!("{label}: {} vs {} blocks", mb.len(), tb.len()),
        );
    } else {
        for (i, (m, t)) in mb.iter().zip(tb).enumerate() {
            if m.uncompressed_size != t.uncompressed_size
                || m.compressed_size != t.compressed_size
                || m.flags as u32 != t.flags
            {
                diff("blocks", format!("{label}: block {i}: {m:?} vs {t:?}"));
            }
            note("block compression", format!("flags {:#x}", m.flags));
        }
    }
    let max_block = mb.iter().map(|b| b.uncompressed_size).max().unwrap_or(0);
    note("max block size", format!("{max_block}"));

    // Entries
    let me = mine.entries();
    let te = theirs.files();
    count("entries", me.len() as u64);
    if me.len() != te.len() {
        diff(
            "entries",
            format!("{label}: {} vs {} entries", me.len(), te.len()),
        );
        return;
    }
    for (m, t) in me.iter().zip(te) {
        if m.path != t.path
            || m.offset as i64 != t.offset
            || m.size as i64 != t.size
            || m.flags != t.flags
        {
            diff("entries", format!("{label}: {m:?} vs {t:?}"));
        }
        note(
            "entry flags",
            format!("flags {:#x} ({})", m.flags, entry_kind(&m.path)),
        );
    }
    let main_m = mine.main_serialized_entry().map(|e| e.path.clone());
    let main_t = theirs.main_serializedfile().map(|e| e.path.clone());
    if main_m != main_t {
        diff(
            "entries",
            format!("{label}: main serialized file {main_m:?} vs {main_t:?}"),
        );
    }

    // The whole decompressed stream, in chunks
    let total: u64 = mb.iter().map(|b| b.uncompressed_size as u64).sum();
    {
        // Facts about the layout
        let mut sorted: Vec<_> = me.iter().collect();
        sorted.sort_by_key(|e| e.offset);
        let mut at = 0u64;
        let mut contiguous = true;
        for e in &sorted {
            contiguous &= e.offset == at;
            at = e.offset + e.size;
        }
        contiguous &= at == total;
        count(
            if contiguous {
                "fact_entries_contiguous_and_cover_the_stream"
            } else {
                "fact_entries_NOT_contiguous"
            },
            1,
        );
        let in_directory_order = me.windows(2).all(|w| w[0].offset <= w[1].offset);
        count(
            if in_directory_order {
                "fact_entries_listed_in_stream_order"
            } else {
                "fact_entries_NOT_in_stream_order"
            },
            1,
        );
        let comp: u64 = mb.iter().map(|b| b.compressed_size as u64).sum();
        let end = mh.block_info_offset + mh.block_info_compressed_size as u64 + comp;
        count(
            if end == bytes.len() as u64 {
                "fact_file_ends_where_the_last_block_ends"
            } else {
                "fact_file_has_trailing_bytes"
            },
            1,
        );
        count(
            if mh.block_info_offset
                == (8
                    + 4
                    + mh.player_version.len()
                    + 1
                    + mh.unity_version.len()
                    + 1
                    + 8
                    + 4
                    + 4
                    + 4) as u64
            {
                "fact_block_info_follows_the_header_fields"
            } else {
                "fact_block_info_elsewhere"
            },
            1,
        );
        let sum_c: u64 = mb
            .iter()
            .filter(|b| b.flags & 0x3f == 0)
            .map(|b| b.uncompressed_size as u64)
            .sum();
        count("fact_bytes_in_stored_blocks", sum_c);
        count(
            "fact_blocks_stored",
            mb.iter().filter(|b| b.flags & 0x3f == 0).count() as u64,
        );
        count(
            "fact_blocks_lz4hc",
            mb.iter().filter(|b| b.flags & 0x3f == 3).count() as u64,
        );
        let streamed = mb.iter().filter(|b| b.flags & !0x3f != 0).count();
        count("fact_blocks_with_flags_beyond_compression", streamed as u64);
    }
    count("stream_bytes", total);
    let whole_m = unity_bundle::bundle::Entry {
        path: "<stream>".into(),
        offset: 0,
        size: total,
        flags: 0,
    };
    let whole_t = rabex::files::unityfile::FileEntry {
        offset: 0,
        size: total as i64,
        flags: 0,
        path: "<stream>".into(),
    };
    let mut pos = 0u64;
    while pos < total {
        let n = ((total - pos) as usize).min(CHUNK);
        let a = mine.read_entry_range(&whole_m, pos, n);
        let b = theirs.read_at_entry_range(&whole_t, pos, n);
        match (a, b) {
            (Ok(a), Ok(b)) => {
                if a != b {
                    let i = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(0);
                    diff(
                        "stream bytes",
                        format!("{label}: differ at {}", pos + i as u64),
                    );
                    return;
                }
            }
            (Err(a), Err(b)) => {
                note(
                    "stream read error in both",
                    format!("{label} at {pos}: {a} / {b}"),
                );
                count("stream_read_error_in_both", 1);
                return;
            }
            (a, b) => {
                diff(
                    "stream read",
                    format!(
                        "{label} at {pos}: mine {:?} theirs {:?}",
                        a.map(|v| v.len()).map_err(|e| e.to_string()),
                        b.map(|v| v.len()).map_err(|e| e.to_string())
                    ),
                );
                return;
            }
        }
        pos += n as u64;
    }
    count("stream_compared_bytes", total);

    // Ranged reads of each entry, mine against theirs
    for (m, t) in me.iter().zip(te) {
        let size = m.size;
        let ranges = [
            (0u64, size.min(64)),
            (size.saturating_sub(64), size.min(64)),
            (size / 2, (size - size / 2).min(1000)),
            (size / 3, (size - size / 3).min(300_000)),
        ];
        for (s, l) in ranges {
            let a = mine.read_entry_range(m, s, l as usize);
            let b = theirs.read_at_entry_range(t, s, l as usize);
            count("entry_ranges_compared", 1);
            match (a, b) {
                (Ok(a), Ok(b)) if a == b => {}
                (Err(_), Err(_)) => count("entry_range_error_in_both", 1),
                (a, b) => diff(
                    "entry range",
                    format!(
                        "{label}: {} start {s} len {l}: mine {:?} theirs {:?}",
                        m.path,
                        a.map(|v| v.len()).map_err(|e| e.to_string()),
                        b.map(|v| v.len()).map_err(|e| e.to_string())
                    ),
                ),
            }
        }
        // out-of-range requests must fail in both
        let a = mine.read_entry_range(m, size, 1);
        let b = theirs.read_at_entry_range(t, size, 1);
        if a.is_ok() != b.is_ok() {
            diff(
                "entry range",
                format!(
                    "{label}: {} one byte past the end: mine ok={} theirs ok={}",
                    m.path,
                    a.is_ok(),
                    b.is_ok()
                ),
            );
        }
    }

    // Whole entries up to 64 MiB: read_entry against read_at_entry
    for (m, t) in me.iter().zip(te) {
        if m.size <= (64 << 20) {
            let a = mine.read_entry(m);
            let b = theirs.read_at_entry(t);
            count("entries_read_whole", 1);
            match (a, b) {
                (Ok(a), Ok(b)) if a == b => {}
                (Err(_), Err(_)) => {}
                (a, b) => diff(
                    "entry whole",
                    format!(
                        "{label}: {}: mine ok={} theirs ok={}",
                        m.path,
                        a.is_ok(),
                        b.is_ok()
                    ),
                ),
            }
        }
    }

    // The main serialized file
    let Some(main_entry) = mine.main_serialized_entry() else {
        count("bundles_without_serialized_file", 1);
        return;
    };
    let mt = theirs.main_serializedfile().unwrap().clone();
    let size = main_entry.size as usize;
    if size > 700 << 20 {
        count("serialized_file_too_large_to_compare", 1);
        note(
            "too large",
            format!("{label}: {} is {} MiB", main_entry.path, size >> 20),
        );
        return;
    }
    let _guard = budget.acquire(size * 2 + (96 << 20));
    let data_m = mine.read_entry(main_entry).unwrap();
    let data_t = theirs.read_at_entry(&mt).unwrap();
    if data_m != data_t {
        diff("serialized bytes", format!("{label}"));
        return;
    }
    count("serialized_files_bytes", size as u64);

    let sm = unity_bundle::SerializedFile::parse(&data_m);
    let st = rabex::files::SerializedFile::from_reader(&mut Cursor::new(&data_t[..]));
    let (sm, st) = match (sm, st) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(a), Err(b)) => {
            count("serialized_parse_error_in_both", 1);
            note(
                "serialized_parse_error_in_both",
                format!("{label}: `{a}` / `{b}`"),
            );
            return;
        }
        (Ok(_), Err(b)) => {
            diff(
                "serialized parse: mine ok, theirs fails",
                format!("{label}: `{b}`"),
            );
            return;
        }
        (Err(a), Ok(_)) => {
            diff(
                "serialized parse: theirs ok, mine fails",
                format!("{label}: `{a}`"),
            );
            return;
        }
    };
    count("serialized_files_parsed_by_both", 1);
    count(
        if sm.file_size == main_entry.size {
            "fact_header_file_size_is_the_entry_size"
        } else {
            "fact_header_file_size_differs"
        },
        1,
    );
    compare_serialized(label, "bundle", &sm, &st);
    compare_values(
        label,
        "bundle",
        &sm,
        &st,
        &data_m,
        &data_t,
        opts,
        Some((&mine, &theirs)),
    );
}

fn entry_kind(path: &str) -> &'static str {
    if path.ends_with(".resS") {
        "resS"
    } else if path.ends_with(".resource") {
        "resource"
    } else if path.ends_with(".sharedAssets") {
        "sharedAssets"
    } else {
        "serialized"
    }
}

// ---------------------------------------------------------------------------------------------
// Serialized file tables

fn compare_tree(
    label: &str,
    what: &str,
    format_version: u32,
    expected_index: &mut i32,
    m: &unity_bundle::TypeTreeNode,
    t: &rabex::typetree::TypeTreeNode,
) -> bool {
    let mut ok = true;
    if m.type_name != t.m_Type
        || m.name != t.m_Name
        || m.version as i32 != t.m_Version
        || m.type_flags as i32 != t.m_TypeFlags
        || m.byte_size != t.m_ByteSize
        || Some(m.meta_flags) != t.m_MetaFlag
        || !(Some(m.ref_type_hash) == t.m_RefTypeHash
            || (format_version < 19 && t.m_RefTypeHash.is_none() && m.ref_type_hash == 0))
        || m.children.len() != t.children.len()
    {
        diff(
            "type tree",
            format!(
                "{label}: {what}: node `{} {}` vs `{} {}`",
                m.type_name, m.name, t.m_Type, t.m_Name
            ),
        );
        return false;
    }
    if m.index != *expected_index {
        diff(
            "type tree index",
            format!(
                "{label}: {what}: node `{}` has index {} but is number {} in pre-order",
                m.name, m.index, expected_index
            ),
        );
    }
    *expected_index += 1;
    for (a, b) in m.children.iter().zip(&t.children) {
        if !compare_tree(label, what, format_version, expected_index, a, b) {
            ok = false;
        }
    }
    ok
}

fn scoped(scope: &str, key: &str, n: u64) {
    count(&format!("{scope}:{key}"), n);
}

fn scoped_if(scope: &str, key: &str, n: u64) {
    count(&format!("{scope}:{key}"), n);
}

fn compare_serialized(
    label: &str,
    scope: &str,
    m: &unity_bundle::SerializedFile,
    t: &rabex::files::SerializedFile,
) {
    let h = &t.m_Header;
    if m.metadata_size != h.m_MetadataSize
        || m.file_size as i64 != h.m_FileSize
        || m.data_offset as i64 != h.m_DataOffset
    {
        diff("serialized header", format!("{label}: sizes"));
    }
    let tv = t
        .m_UnityVersion
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_default();
    if m.unity_version != tv {
        diff(
            "serialized header",
            format!("{label}: unity version {:?} vs {tv:?}", m.unity_version),
        );
    }
    if Some(m.target_platform) != t.m_TargetPlatform {
        diff("serialized header", format!("{label}: target platform"));
    }
    if m.has_type_tree != t.m_EnableTypeTree {
        diff("serialized header", format!("{label}: type tree flag"));
    }
    note(
        "serialized header",
        format!(
            "format {} (mine {}), platform {}, type tree {}, unity {}",
            h.m_Version, m.format_version, m.target_platform, m.has_type_tree, m.unity_version
        ),
    );

    // Types
    count("types", m.types.len() as u64);
    {
        let ids: std::collections::HashSet<i64> = m.objects.iter().map(|o| o.path_id).collect();
        scoped_if(
            scope,
            if ids.len() == m.objects.len() {
                "fact_path_ids_unique"
            } else {
                "fact_path_ids_REPEATED"
            },
            1,
        );
        let mut by_off: Vec<_> = m.objects.iter().collect();
        by_off.sort_by_key(|o| o.offset);
        let rel_ok = m
            .objects
            .iter()
            .all(|o| (o.offset - m.data_offset) % 8 == 0);
        scoped_if(
            scope,
            if rel_ok {
                "fact_object_offsets_8_aligned_from_data_offset"
            } else {
                "fact_object_offsets_NOT_8_aligned"
            },
            1,
        );
        let mut disjoint = true;
        for w in by_off.windows(2) {
            disjoint &= w[0].offset + w[0].size as u64 <= w[1].offset;
        }
        scoped_if(
            scope,
            if disjoint {
                "fact_objects_disjoint"
            } else {
                "fact_objects_OVERLAP"
            },
            1,
        );
        scoped_if(
            scope,
            if m.data_offset % 16 == 0 {
                "fact_data_offset_16_aligned"
            } else {
                "fact_data_offset_NOT_16_aligned"
            },
            1,
        );
        scoped_if(
            scope,
            if m.data_offset == (20 + m.metadata_size as u64).div_ceil(16) * 16 {
                "fact_data_offset_is_header_plus_metadata_rounded_to_16"
            } else {
                "fact_data_offset_other"
            },
            1,
        );
        scoped_if(
            scope,
            if m.user_information.is_empty() {
                "fact_user_information_empty"
            } else {
                "fact_user_information_NOT_empty"
            },
            1,
        );
        scoped(scope, "fact_script_types", m.script_types.len() as u64);
        scoped(
            scope,
            "fact_stripped_types",
            m.types.iter().filter(|t| t.is_stripped).count() as u64,
        );
        scoped(
            scope,
            "fact_types_with_script_id",
            m.types.iter().filter(|t| t.script_id != [0; 16]).count() as u64,
        );
        scoped(
            scope,
            "fact_types_with_dependencies",
            m.types
                .iter()
                .filter(|t| !t.type_dependencies.is_empty())
                .count() as u64,
        );
        scoped(
            scope,
            "fact_types_script_type_index_not_minus1",
            m.types.iter().filter(|t| t.script_type_index != -1).count() as u64,
        );
        scoped(
            scope,
            "fact_externals_kind_nonzero",
            m.externals.iter().filter(|e| e.kind != 0).count() as u64,
        );
        scoped(
            scope,
            "fact_externals_guid_nonzero",
            m.externals.iter().filter(|e| e.guid != [0; 16]).count() as u64,
        );
        scoped(
            scope,
            "fact_externals_temp_empty_nonempty",
            m.externals
                .iter()
                .filter(|e| !e.temp_empty.is_empty())
                .count() as u64,
        );
        if m.has_type_tree {
            scoped_if(
                scope,
                if m.types.iter().all(|t| t.type_tree.is_some()) {
                    "fact_every_type_has_a_tree"
                } else {
                    "fact_some_type_without_tree"
                },
                1,
            );
        }
    }
    if m.types.len() != t.m_Types.len() {
        diff(
            "types",
            format!("{label}: {} vs {} types", m.types.len(), t.m_Types.len()),
        );
    } else {
        for (i, (a, b)) in m.types.iter().zip(&t.m_Types).enumerate() {
            if a.class_id.0 != b.m_ClassID.0
                || a.is_stripped != b.m_IsStrippedType
                || a.script_type_index != b.m_ScriptTypeIndex
                || a.script_id != b.m_ScriptID
                || a.old_type_hash != b.m_OldTypeHash
                || a.type_dependencies != b.m_TypeDependencies
                || a.reference_names.is_some()
                || a.type_tree.is_some() != b.m_Type.is_some()
            {
                diff("types", format!("{label}: type {i}"));
            }
            if let (Some(x), Some(y)) = (&a.type_tree, &b.m_Type) {
                let mut idx = 0;
                count("type_tree_nodes", node_count(x));
                tree_facts(x);
                if compare_tree(
                    label,
                    &format!("type {i} ({})", a.class_id.0),
                    m.format_version,
                    &mut idx,
                    x,
                    y,
                ) {
                    count("type_trees_identical", 1);
                    if x.dump() != y.dump() {
                        diff("type tree dump", format!("{label}: type {i}"));
                    }
                }
            }
            // Facts about the table
            if let Some(x) = &a.type_tree {
                if let Some(n) = a.class_id.name() {
                    if n != x.type_name && a.class_id.0 != 114 {
                        note(
                            "class name differs from the root type",
                            format!(
                                "class {} named {n} but root type is {}",
                                a.class_id.0, x.type_name
                            ),
                        );
                    }
                } else {
                    note(
                        "class id without a name in mine",
                        format!("{} root {}", a.class_id.0, x.type_name),
                    );
                }
            }
        }
    }
    // Class names against theirs, for every id met
    for ty in &m.types {
        let mine = ty.class_id.name().map(str::to_string);
        let theirs = rabex::objects::ClassId(ty.class_id.0)
            .name()
            .map(str::to_string);
        if mine != theirs {
            diff(
                "class names",
                format!(
                    "class {}: mine {:?} theirs {:?}",
                    ty.class_id.0, mine, theirs
                ),
            );
        }
    }

    // Objects
    count("objects", m.objects.len() as u64);
    let to: Vec<&rabex::files::serializedfile::ObjectInfo> = t.objects().collect();
    if m.objects.len() != to.len() {
        diff(
            "objects",
            format!("{label}: {} vs {} objects", m.objects.len(), to.len()),
        );
    } else {
        for (a, b) in m.objects.iter().zip(&to) {
            if a.path_id != b.m_PathID
                || a.offset as i64 != b.m_Offset
                || a.size != b.m_Size
                || a.type_index != b.m_TypeID
                || a.class_id.0 != b.m_ClassID.0
            {
                diff("objects", format!("{label}: object {} differs", a.path_id));
            }
            let la = m.object(a.path_id).map(|o| (o.offset, o.size));
            let lb = t
                .get_object_info(a.path_id)
                .map(|o| (o.m_Offset as u64, o.m_Size));
            if la != lb {
                diff(
                    "objects",
                    format!("{label}: lookup of {} differs", a.path_id),
                );
            }
        }
    }
    // Externals, scripts, refs
    if m.externals.len() != t.m_Externals.len() {
        diff("externals", format!("{label}: count"));
    } else {
        for (a, b) in m.externals.iter().zip(&t.m_Externals) {
            if Some(&a.temp_empty) != b.tempEmpty.as_ref()
                || Some(a.guid) != b.guid.as_ref().map(|g| g.0)
                || Some(a.kind) != b.typeId
                || a.path != b.pathName
            {
                diff("externals", format!("{label}: {a:?} vs {b:?}"));
            }
        }
    }
    count("externals", m.externals.len() as u64);
    let ts = t.m_ScriptTypes.as_deref().unwrap_or(&[]);
    if m.script_types.len() != ts.len() {
        diff("script types", format!("{label}: count"));
    } else {
        for (a, b) in m.script_types.iter().zip(ts) {
            if format!("{:?}", b.m_LocalSerializedFileIndex) != a.file_index.to_string()
                || b.m_LocalIdentifierInFile != a.path_id
            {
                diff("script types", format!("{label}: entry"));
            }
        }
    }
    let tr = t.m_RefTypes.as_deref().unwrap_or(&[]);
    if m.ref_types.len() != tr.len() {
        diff(
            "ref types",
            format!("{label}: count {} vs {}", m.ref_types.len(), tr.len()),
        );
    }
    count("ref_types", m.ref_types.len() as u64);
    if Some(&m.user_information) != t.m_UserInformation.as_ref() {
        diff("serialized header", format!("{label}: user information"));
    }
}

fn tree_facts(n: &unity_bundle::TypeTreeNode) {
    let a = |x: &unity_bundle::TypeTreeNode| x.aligns() as u8;
    let one_array = n.children.len() == 1 && n.children[0].type_name == "Array";
    match n.type_name.as_str() {
        "string" => count(
            &format!(
                "fact_string_align_node{}_array{}",
                a(n),
                n.children.first().map(a).unwrap_or(9)
            ),
            1,
        ),
        "TypelessData" => count(&format!("fact_typeless_align_node{}", a(n)), 1),
        "map" => {
            let arr = n.children.first();
            let pair = arr.and_then(|x| x.children.get(1));
            count(
                &format!(
                    "fact_map_align_node{}_array{}_pair{}",
                    a(n),
                    arr.map(a).unwrap_or(9),
                    pair.map(a).unwrap_or(9)
                ),
                1,
            );
        }
        "Array" | "bool" | "UInt8" | "SInt8" | "char" | "UInt16" | "SInt16" | "unsigned short"
        | "short" | "UInt32" | "SInt32" | "int" | "unsigned int" | "float" | "UInt64"
        | "SInt64" | "double" | "unsigned long long" | "long long" | "FileSize" | "Type*" => {
            if a(n) == 1 {
                count(
                    &format!("fact_primitive_aligned_{}", n.type_name.replace(' ', "_")),
                    1,
                );
            }
        }
        _ if one_array => {
            let arr = &n.children[0];
            count(
                &format!("fact_vector_align_node{}_array{}", a(n), a(arr)),
                1,
            );
        }
        _ if !n.children.is_empty() => {
            count(&format!("fact_struct_align_node{}", a(n)), 1);
        }
        _ => {}
    }
    for c in &n.children {
        tree_facts(c);
    }
}

fn node_count(n: &unity_bundle::TypeTreeNode) -> u64 {
    1 + n.children.iter().map(node_count).sum::<u64>()
}

// ---------------------------------------------------------------------------------------------
// Values

type Theirs<'a> = &'a rabex::files::bundlefile::BundleFileReader<Cursor<&'a [u8]>>;
type Mine<'a> = &'a unity_bundle::Bundle<&'a [u8]>;

fn compare_values(
    label: &str,
    scope: &str,
    sm: &unity_bundle::SerializedFile,
    st: &rabex::files::SerializedFile,
    data_m: &[u8],
    data_t: &[u8],
    opts: &Options,
    bundles: Option<(Mine, Theirs)>,
) {
    if !opts.values {
        return;
    }
    let mut cur_t = Cursor::new(data_t);
    for info in &sm.objects {
        let Some(tinfo) = st.get_object_info(info.path_id) else {
            continue;
        };
        let class = info.class_id.0;
        let tt_m = sm.type_tree(info);
        count("objects_considered", 1);
        if tt_m.is_none() {
            // No type tree: both must refuse
            let b = st.read::<Dig>(tinfo, rabex::typetree::NullTypeTreeProvider, &mut cur_t);
            let a = sm.read::<Dig>(data_m, info);
            count("objects_without_type_tree", 1);
            if a.is_ok() || b.is_ok() {
                diff(
                    "values",
                    format!("{label}: object {} without type tree read ok", info.path_id),
                );
            }
            continue;
        }

        // Generic digest through deserialize_any
        let start = reset_hash();
        let a =
            unity_bundle::de::from_slice::<Dig>(sm.raw(data_m, info).unwrap_or(&[]), tt_m.unwrap());
        let a = digest_of(a.map(|_| Dig), start);
        let start = reset_hash();
        let b = st.read::<Dig>(tinfo, rabex::typetree::NullTypeTreeProvider, &mut cur_t);
        let b = digest_of(b, start);
        count("object_digests_compared", 1);
        count(&format!("class_{class}_objects"), 1);
        count(&format!("{scope}:objects of class {class}"), 1);
        match (&a, &b) {
            (Outcome::Ok(x, n), Outcome::Ok(y, m)) => {
                if x == y && n == m {
                    count(&format!("class_{class}_digest_identical"), 1);
                } else {
                    diff(
                        "digest",
                        format!(
                            "{label}: object {} class {class}: events {n} vs {m}",
                            info.path_id
                        ),
                    );
                }
            }
            (Outcome::Err(x), Outcome::Err(y)) => {
                count("object_digest_error_in_both", 1);
                count(&format!("class_{class}_digest_error_in_both"), 1);
                note(
                    "digest error in both",
                    format!("class {class}: mine `{}` / theirs `{}`", short(x), short(y)),
                );
            }
            (Outcome::Ok(..), Outcome::Err(y)) => diff(
                "digest: mine ok, theirs error",
                format!(
                    "{label}: object {} class {class}: theirs `{}`",
                    info.path_id,
                    short(y)
                ),
            ),
            (Outcome::Err(x), Outcome::Ok(..)) => diff(
                "digest: theirs ok, mine error",
                format!(
                    "{label}: object {} class {class}: mine `{}`",
                    info.path_id,
                    short(x)
                ),
            ),
        }

        // Typed values for the classes nocturne-unpack reads
        macro_rules! typed {
            ($t:ty) => {
                typed_compare::<$t>(label, sm, st, data_m, &mut cur_t, info, tinfo, bundles)
            };
        }
        match class {
            1 => typed!(GameObject),
            4 => typed!(Transform),
            21 => typed!(Material),
            23 | 137 => typed!(Renderer),
            33 => typed!(MeshFilter),
            43 => typed!(Mesh),
            28 => typed!(Texture2D),
            48 => typed!(Shader),
            49 => typed!(TextAsset),
            74 => typed!(AnimationClip),
            90 => typed!(Avatar),
            91 => typed!(AnimatorController),
            95 => typed!(Animator),
            142 => typed!(AssetBundle),
            _ => {}
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(110).collect()
}

trait StreamOf {
    fn stream(&self) -> Option<&StreamingInfo>;
}
impl StreamOf for Texture2D {
    fn stream(&self) -> Option<&StreamingInfo> {
        Some(&self.m_StreamData)
    }
}
impl StreamOf for Mesh {
    fn stream(&self) -> Option<&StreamingInfo> {
        Some(&self.m_StreamData)
    }
}
macro_rules! no_stream {
    ($($t:ty),*) => {$(impl StreamOf for $t { fn stream(&self) -> Option<&StreamingInfo> { None } })*};
}
no_stream!(
    GameObject,
    Transform,
    Material,
    Renderer,
    MeshFilter,
    Shader,
    TextAsset,
    AnimationClip,
    Avatar,
    AnimatorController,
    Animator,
    AssetBundle
);

#[allow(clippy::too_many_arguments)]
fn typed_compare<T: DeserializeOwned + Debug + StreamOf>(
    label: &str,
    sm: &unity_bundle::SerializedFile,
    st: &rabex::files::SerializedFile,
    data_m: &[u8],
    cur_t: &mut Cursor<&[u8]>,
    info: &unity_bundle::serialized::ObjectInfo,
    tinfo: &rabex::files::serializedfile::ObjectInfo,
    bundles: Option<(Mine, Theirs)>,
) {
    let name = std::any::type_name::<T>().rsplit("::").next().unwrap();
    let a = sm.read::<T>(data_m, info);
    let b = st.read::<T>(tinfo, rabex::typetree::NullTypeTreeProvider, cur_t);
    count(&format!("typed_{name}_compared"), 1);
    match (a, b) {
        (Ok(a), Ok(b)) => {
            if debug_hash(&a) == debug_hash(&b) {
                count(&format!("typed_{name}_identical"), 1);
            } else {
                let (x, y) = (format!("{a:?}"), format!("{b:?}"));
                diff(
                    "typed",
                    format!(
                        "{label}: {name} object {}: {}",
                        info.path_id,
                        first_difference(&x, &y)
                    ),
                );
            }
            if name == "Texture2D" {
                // (only to count facts: the image data sizes that are not a multiple of 4)
                if let Ok(t) = sm.read::<Texture2D>(data_m, info) {
                    if t.image_data.len() % 4 != 0 {
                        count("fact_texture_image_data_len_not_multiple_of_4", 1);
                        let at = info.offset as usize + info.size as usize;
                        note(
                            "textures with image data of a length that is not a multiple of 4",
                            format!(
                                "{label} {}: {}x{} format {} image data {} bytes, object size {}, m_StreamData {:?}, last 16 bytes of the object {:02x?}",
                                t.m_Name, t.m_Width, t.m_Height, t.m_TextureFormat, t.image_data.len(), info.size, t.m_StreamData,
                                &data_m[at.saturating_sub(16)..at]
                            ),
                        );
                    }
                    if !t.image_data.is_empty() {
                        count("fact_texture_with_image_data_in_object", 1);
                    }
                }
            }
            // The streamed resource of textures and meshes
            if let (Some(s), Some((bm, bt))) = (a.stream(), bundles) {
                compare_stream(label, name, s, bm, bt);
            }
        }
        (Err(x), Err(y)) => {
            count(&format!("typed_{name}_error_in_both"), 1);
            note(
                "typed error in both",
                format!(
                    "{name}: mine `{}` / theirs `{}`",
                    short(&x.to_string()),
                    short(&y.to_string())
                ),
            );
        }
        (Ok(_), Err(y)) => diff(
            "typed: mine ok, theirs error",
            format!(
                "{label}: {name} object {}: theirs `{}`",
                info.path_id,
                short(&y.to_string())
            ),
        ),
        (Err(x), Ok(_)) => diff(
            "typed: theirs ok, mine error",
            format!(
                "{label}: {name} object {}: mine `{}`",
                info.path_id,
                short(&x.to_string())
            ),
        ),
    }
}

fn compare_stream(label: &str, name: &str, s: &StreamingInfo, bm: Mine, bt: Theirs) {
    if s.size == 0 && s.path.is_empty() {
        return;
    }
    let file = s.path.rsplit('/').next().unwrap_or(&s.path);
    let em = bm.entries().iter().find(|e| e.path == file);
    let et = bt.files().iter().find(|e| e.path == file);
    count("streams_compared", 1);
    match (em, et) {
        (Some(em), Some(et)) => {
            let a = bm.read_entry_range(em, s.offset as u64, s.size as usize);
            let b = bt.read_at_entry_range(et, s.offset as u64, s.size as usize);
            match (a, b) {
                (Ok(a), Ok(b)) if a == b => {
                    count("stream_bytes_identical", a.len() as u64);
                }
                (Err(_), Err(_)) => count("stream_error_in_both", 1),
                (a, b) => diff(
                    "stream",
                    format!(
                        "{label}: {name} {}: mine ok={} theirs ok={}",
                        s.path,
                        a.is_ok(),
                        b.is_ok()
                    ),
                ),
            }
        }
        (None, None) => count("stream_entry_missing_in_both", 1),
        _ => diff(
            "stream",
            format!("{label}: {name}: entry {file} found by one only"),
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Loose serialized files (levels, .assets, globalgamemanagers)

fn process_loose(path: &Path, label: &str, opts: &Options, budget: &Budget) {
    count("loose_files_total", 1);
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0) as usize;
    if size > 700 << 20 {
        note("loose too large", format!("{label} {} MiB", size >> 20));
        return;
    }
    let _g = budget.acquire(size + (32 << 20));
    let data = std::fs::read(path).unwrap();
    let sm = unity_bundle::SerializedFile::parse(&data);
    let st = rabex::files::SerializedFile::from_reader(&mut Cursor::new(&data[..]));
    match (sm, st) {
        (Ok(a), Ok(b)) => {
            count("loose_parsed_by_both", 1);
            compare_serialized(label, "loose", &a, &b);
            compare_values(label, "loose", &a, &b, &data, &data, opts, None);
        }
        (Err(a), Err(b)) => {
            count("loose_error_in_both", 1);
            note(
                "loose error in both",
                format!("{label}: mine `{a}` / theirs `{b}`"),
            );
        }
        (Ok(_), Err(b)) => diff("loose: mine ok, theirs fails", format!("{label}: `{b}`")),
        (Err(a), Ok(_)) => diff("loose: theirs ok, mine fails", format!("{label}: `{a}`")),
    }
}

// ---------------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------------
// Hostile input: mutated copies of real files

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
                if v.len() >= 4 {
                    let i = rng.below(v.len() - 3);
                    let val = [0x7FFF_FFFFu32, 0x8000_0000, 0x00FF_FFFF, 0xFFFF_FFF0][rng.below(4)];
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

/// Everything a user does with a serialized file, on bytes that may be garbage.
fn exercise_serialized(data: &[u8]) -> bool {
    let Ok(f) = unity_bundle::SerializedFile::parse(data) else {
        return false;
    };
    for info in f.objects.iter().take(3000) {
        let _ = f.raw(data, info);
        let _ = f.read::<Dig>(data, info);
        match info.class_id.0 {
            1 => drop(f.read::<GameObject>(data, info)),
            4 => drop(f.read::<Transform>(data, info)),
            21 => drop(f.read::<Material>(data, info)),
            28 => drop(f.read::<Texture2D>(data, info)),
            43 => drop(f.read::<Mesh>(data, info)),
            74 => drop(f.read::<AnimationClip>(data, info)),
            137 => drop(f.read::<Renderer>(data, info)),
            _ => {}
        }
    }
    true
}

/// Everything a user does with a bundle, on bytes that may be garbage.
fn exercise(bytes: &[u8]) -> (bool, bool) {
    let Ok(b) = unity_bundle::Bundle::new(bytes) else {
        return (false, false);
    };
    let mut parsed = false;
    for e in b.entries() {
        let _ = b.read_entry_range(e, 0, (e.size as usize).min(1 << 16));
        let _ = b.read_entry_range(e, e.size / 2, 100);
        if e.size < (64 << 20) {
            if let Ok(data) = b.read_entry(e) {
                if e.is_serialized() {
                    parsed |= exercise_serialized(&data);
                }
            }
        }
    }
    (true, parsed)
}

fn fuzz(game: &Path, per_bundle: usize, max_file: u64, max_bundles: usize) {
    let pc = game.join("smt3hd_Data/StreamingAssets/PC");
    let mut files: Vec<PathBuf> = files_in(&pc)
        .into_iter()
        .filter(|p| p.metadata().map(|m| m.len() <= max_file).unwrap_or(false))
        .collect();
    // a spread over the names rather than the first ones
    let step = (files.len() / max_bundles.max(1)).max(1);
    files = files.into_iter().step_by(step).take(max_bundles).collect();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let (mut execs, mut opened, mut parsed, mut panics) = (0u64, 0u64, 0u64, 0u64);
    let mut largest_seen = 0usize;
    let mut worst = String::new();
    let (mut sexecs, mut sparsed) = (0u64, 0u64);
    for p in &files {
        let base = std::fs::read(p).unwrap();
        // the decompressed serialized file of the bundle, mutated (compressed data would mostly
        // fail to decompress before the parser is reached)
        if let Ok(b) = unity_bundle::Bundle::new(&base[..]) {
            if let Some(e) = b.main_serialized_entry() {
                if e.size < (8 << 20) {
                    if let Ok(plain) = b.read_entry(e) {
                        for k in 0..per_bundle {
                            let m = mutate(&mut rng, &plain);
                            LARGEST.store(0, Ordering::Relaxed);
                            TRACK.store(true, Ordering::Relaxed);
                            let r = std::panic::catch_unwind(|| exercise_serialized(&m));
                            TRACK.store(false, Ordering::Relaxed);
                            sexecs += 1;
                            match r {
                                Ok(pa) => sparsed += pa as u64,
                                Err(_) => {
                                    panics += 1;
                                    diff(
                                        "PANIC on mutated serialized file",
                                        format!("{} mutant {k}", p.display()),
                                    );
                                }
                            }
                            let l = LARGEST.load(Ordering::Relaxed);
                            if l > largest_seen {
                                largest_seen = l;
                                worst = format!(
                                    "{} serialized mutant {k}",
                                    p.file_name().unwrap().to_string_lossy()
                                );
                            }
                        }
                    }
                }
            }
        }
        for k in 0..per_bundle {
            let m = mutate(&mut rng, &base);
            LARGEST.store(0, Ordering::Relaxed);
            TRACK.store(true, Ordering::Relaxed);
            let r = std::panic::catch_unwind(|| exercise(&m));
            TRACK.store(false, Ordering::Relaxed);
            execs += 1;
            match r {
                Ok((o, pa)) => {
                    opened += o as u64;
                    parsed += pa as u64;
                }
                Err(_) => {
                    panics += 1;
                    diff(
                        "PANIC on mutated bundle",
                        format!("{} mutant {k}", p.display()),
                    );
                }
            }
            let l = LARGEST.load(Ordering::Relaxed);
            if l > largest_seen {
                largest_seen = l;
                worst = format!("{} mutant {k}", p.file_name().unwrap().to_string_lossy());
            }
        }
    }
    println!(
        "== hostile-input pass: {} bundles of at most {} KiB, {} mutants each",
        files.len(),
        max_file >> 10,
        per_bundle
    );
    println!("mutants run: {execs}; still opened as bundles: {opened}; main serialized file still parsed: {parsed}; panics: {panics}");
    println!("serialized-file mutants run: {sexecs}; still parsed: {sparsed}");
    println!(
        "largest single allocation: {} bytes ({})",
        largest_seen, worst
    );
}

/// Times the decompression of a whole bundle with both libraries.
fn bench(path: &str) {
    let file = std::fs::File::open(path).unwrap();
    let map = unsafe { Mmap::map(&file) }.unwrap();
    let bytes: &[u8] = &map[..];
    let mine = unity_bundle::Bundle::new(bytes).unwrap();
    let theirs = rabex::files::bundlefile::BundleFileReader::from_reader(
        Cursor::new(bytes),
        &rabex_config(),
    )
    .unwrap();
    let total: u64 = mine
        .blocks()
        .iter()
        .map(|b| b.uncompressed_size as u64)
        .sum();
    let ent_m = unity_bundle::bundle::Entry {
        path: "s".into(),
        offset: 0,
        size: total,
        flags: 0,
    };
    let ent_t = rabex::files::unityfile::FileEntry {
        offset: 0,
        size: total as i64,
        flags: 0,
        path: "s".into(),
    };
    // serialized file parse
    let main = mine.main_serialized_entry().unwrap().clone();
    let data = mine.read_entry(&main).unwrap();
    for round in 0..3 {
        let t = std::time::Instant::now();
        let mut n = 0;
        for _ in 0..20 {
            n += unity_bundle::SerializedFile::parse(&data)
                .unwrap()
                .objects
                .len();
        }
        let a = t.elapsed();
        let t = std::time::Instant::now();
        for _ in 0..20 {
            n += rabex::files::SerializedFile::from_reader(&mut Cursor::new(&data[..]))
                .unwrap()
                .objects()
                .len();
        }
        let b = t.elapsed();
        println!(
            "parse x20 round {round}: mine {:.1} ms, rabex {:.1} ms [{n}]",
            a.as_secs_f64() * 1e3,
            b.as_secs_f64() * 1e3
        );
    }
    for round in 0..3 {
        let t = std::time::Instant::now();
        let mut pos = 0;
        let mut sum = 0u64;
        while pos < total {
            let n = ((total - pos) as usize).min(CHUNK);
            sum += mine.read_entry_range(&ent_m, pos, n).unwrap().len() as u64;
            pos += n as u64;
        }
        let a = t.elapsed();
        let t = std::time::Instant::now();
        let mut pos = 0;
        while pos < total {
            let n = ((total - pos) as usize).min(CHUNK);
            sum += theirs.read_at_entry_range(&ent_t, pos, n).unwrap().len() as u64;
            pos += n as u64;
        }
        let b = t.elapsed();
        println!("round {round}: {} MB decompressed; mine {:.0} ms ({:.2} GB/s), rabex {:.0} ms ({:.2} GB/s) [{sum}]", total >> 20, a.as_secs_f64() * 1e3, total as f64 / a.as_secs_f64() / 1e9, b.as_secs_f64() * 1e3, total as f64 / b.as_secs_f64() / 1e9);
    }
}

fn rss_mb() -> f64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    #[cfg(target_os = "macos")]
    return ru.ru_maxrss as f64 / 1048576.0;
    #[cfg(not(target_os = "macos"))]
    return ru.ru_maxrss as f64 / 1024.0;
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn main() {
    let mut args = std::env::args().skip(1);
    let game = PathBuf::from(args.next().expect("usage: oracle GAME_DIR [--threads N] [--budget-mb N] [--only TEXT] [--no-values] [--bundles-only]"));
    let mut threads = 3usize;
    let mut budget_mb = 1400usize;
    let mut only = None;
    let mut values = true;
    let mut loose = true;
    let mut bundles_flag = true;
    let mut fuzz_per_bundle = 0usize;
    let mut fuzz_max_kb = 600u64;
    let mut fuzz_bundles = 250usize;
    let mut bench_file = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bench" => bench_file = args.next(),
            "--fuzz" => fuzz_per_bundle = args.next().unwrap().parse().unwrap(),
            "--fuzz-max-kb" => fuzz_max_kb = args.next().unwrap().parse().unwrap(),
            "--fuzz-bundles" => fuzz_bundles = args.next().unwrap().parse().unwrap(),
            "--threads" => threads = args.next().unwrap().parse().unwrap(),
            "--budget-mb" => budget_mb = args.next().unwrap().parse().unwrap(),
            "--only" => only = args.next(),
            "--no-values" => values = false,
            "--bundles-only" => loose = false,
            "--loose-only" => bundles_flag = false,
            other => panic!("unknown option {other}"),
        }
    }
    let opts = Options { values, only };
    if let Some(b) = bench_file {
        bench(&b);
        return;
    }
    if fuzz_per_bundle > 0 {
        fuzz(&game, fuzz_per_bundle, fuzz_max_kb << 10, fuzz_bundles);
        let rep = REPORT.lock().unwrap().take().unwrap_or_default();
        for (k, v) in &rep.diff_counts {
            println!("[{k}] {v}");
            for s in rep.diff_samples.get(k).into_iter().flatten() {
                println!("  {s}");
            }
        }
        println!("peak RSS {:.0} MB", rss_mb());
        return;
    }

    // Work list: (path, label, loose)
    let mut work: Vec<(PathBuf, String, bool)> = Vec::new();
    if bundles_flag {
        let pc = game.join("smt3hd_Data/StreamingAssets/PC");
        for p in files_in(&pc) {
            let l = format!("PC/{}", p.file_name().unwrap().to_string_lossy());
            work.push((p, l, false));
        }
        if let Ok(rd) = std::fs::read_dir(game.join("dlc")) {
            let mut dirs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            for d in dirs {
                for p in files_in(&d) {
                    let l = format!(
                        "dlc/{}/{}",
                        d.file_name().unwrap().to_string_lossy(),
                        p.file_name().unwrap().to_string_lossy()
                    );
                    work.push((p, l, false));
                }
            }
        }
    }
    if loose {
        for p in files_in(&game.join("smt3hd_Data")) {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            if n.starts_with("level") && !n.ends_with(".resS")
                || n.ends_with(".assets")
                || n == "globalgamemanagers"
            {
                work.push((p, format!("Data/{n}"), true));
            }
        }
    }
    if let Some(o) = &opts.only {
        work.retain(|(_, l, _)| l.contains(o.as_str()));
    }
    eprintln!(
        "{} files, {} threads, budget {} MB",
        work.len(),
        threads,
        budget_mb
    );

    let budget = Budget {
        used: Mutex::new(0),
        cv: Condvar::new(),
        cap: budget_mb << 20,
    };
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let started = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((path, label, is_loose)) = work.get(i) else {
                    break;
                };
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if *is_loose {
                        process_loose(path, label, &opts, &budget)
                    } else {
                        process_bundle(path, label, &opts, &budget)
                    }
                }));
                if r.is_err() {
                    diff("PANIC", format!("{label}"));
                }
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                if d % 100 == 0 {
                    eprintln!(
                        "{d}/{} done, {:.0}s, peak RSS {:.0} MB",
                        work.len(),
                        started.elapsed().as_secs_f64(),
                        rss_mb()
                    );
                }
            });
        }
    });

    let rep = REPORT.lock().unwrap().take().unwrap_or_default();
    println!(
        "== oracle report: {} files in {:.0}s, peak RSS {:.0} MB",
        work.len(),
        started.elapsed().as_secs_f64(),
        rss_mb()
    );
    println!("\n-- counts");
    for (k, v) in &rep.counts {
        println!("{k}: {v}");
    }
    println!("\n-- notes (distinct values seen)");
    for (k, v) in &rep.notes {
        println!("[{k}]");
        for x in v {
            println!("  {x}");
        }
    }
    println!("\n-- differences");
    if rep.diff_counts.is_empty() {
        println!("none");
    }
    for (k, v) in &rep.diff_counts {
        println!("[{k}] {v}");
        for s in rep.diff_samples.get(k).into_iter().flatten() {
            println!("  {s}");
        }
    }
}
