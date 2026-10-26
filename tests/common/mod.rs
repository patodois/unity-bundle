//! Writers for small bundles and serialized files, so that the tests need no game files.
//!
//! The layouts are the ones in `docs/FORMATS.md`; the writers are test code, not part of the
//! crate, and write strings into each type tree's own buffer (never the common-string table).

#![allow(dead_code)]

/// A node of a type tree to write.
#[derive(Clone)]
pub struct N {
    pub ty: &'static str,
    pub name: &'static str,
    pub meta: i32,
    pub children: Vec<N>,
}

pub const ALIGN: i32 = 1 << 14;

pub fn n(ty: &'static str, name: &'static str, children: Vec<N>) -> N {
    N {
        ty,
        name,
        meta: 0,
        children,
    }
}

/// A node that pads to 4 bytes after it.
pub fn na(ty: &'static str, name: &'static str, children: Vec<N>) -> N {
    N {
        meta: ALIGN,
        ..n(ty, name, children)
    }
}

pub fn prim(ty: &'static str, name: &'static str) -> N {
    n(ty, name, vec![])
}

/// `string name`: a length, the bytes, padding.
pub fn string(name: &'static str) -> N {
    n(
        "string",
        name,
        vec![na(
            "Array",
            "Array",
            vec![prim("int", "size"), prim("char", "data")],
        )],
    )
}

/// `vector name` of items of one node.
pub fn vector(name: &'static str, item: N) -> N {
    n(
        "vector",
        name,
        vec![na(
            "Array",
            "Array",
            vec![
                prim("int", "size"),
                N {
                    name: "data",
                    ..item
                },
            ],
        )],
    )
}

/// `map name` from keys to values.
pub fn map(name: &'static str, key: N, value: N) -> N {
    n(
        "map",
        name,
        vec![n(
            "Array",
            "Array",
            vec![
                prim("int", "size"),
                n(
                    "pair",
                    "data",
                    vec![
                        N {
                            name: "first",
                            ..key
                        },
                        N {
                            name: "second",
                            ..value
                        },
                    ],
                ),
            ],
        )],
    )
}

pub fn typeless(name: &'static str) -> N {
    n(
        "TypelessData",
        name,
        vec![prim("int", "size"), prim("UInt8", "data")],
    )
}

pub fn pptr(name: &'static str) -> N {
    n(
        "PPtr<Object>",
        name,
        vec![prim("int", "m_FileID"), prim("SInt64", "m_PathID")],
    )
}

// ---------------------------------------------------------------------------------------------
// Data writers

#[derive(Default)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    pub fn cstr(&mut self, s: &str) -> &mut Self {
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
        self
    }
    /// A string value of a type tree: length, bytes, padding to 4
    pub fn string(&mut self, s: &str) -> &mut Self {
        self.u32(s.len() as u32).bytes(s.as_bytes()).align4()
    }
    pub fn align4(&mut self) -> &mut Self {
        while self.0.len() % 4 != 0 {
            self.0.push(0);
        }
        self
    }
    pub fn be_u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn be_i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn be_u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn be_i64(&mut self, v: i64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
}

// ---------------------------------------------------------------------------------------------
// Type tree blob

/// The type tree blob of format version `version` (17 gives 24-byte records, 19 and later 32).
pub fn tree_blob(root: &N, version: u32) -> Vec<u8> {
    let mut nodes = Vec::new();
    let mut strings = Vec::new();
    fn string_offset(strings: &mut Vec<u8>, s: &str) -> u32 {
        let at = strings.len() as u32;
        strings.extend_from_slice(s.as_bytes());
        strings.push(0);
        at
    }
    fn walk(
        n: &N,
        level: u8,
        nodes: &mut Vec<u8>,
        strings: &mut Vec<u8>,
        count: &mut i32,
        version: u32,
    ) {
        let mut w = W::default();
        w.u16(1).u8(level).u8(0);
        let t = string_offset(strings, n.ty);
        let nm = string_offset(strings, n.name);
        w.u32(t).u32(nm).i32(-1).i32(*count).i32(n.meta);
        if version >= 19 {
            w.u64(0);
        }
        nodes.extend_from_slice(&w.0);
        *count += 1;
        for c in &n.children {
            walk(c, level + 1, nodes, strings, count, version);
        }
    }
    let mut count = 0;
    walk(root, 0, &mut nodes, &mut strings, &mut count, version);
    let mut w = W::default();
    w.i32(count)
        .i32(strings.len() as i32)
        .bytes(&nodes)
        .bytes(&strings);
    w.0
}

// ---------------------------------------------------------------------------------------------
// Serialized file

pub struct TypeSpec {
    pub class_id: i32,
    pub tree: Option<N>,
}

pub struct ObjectSpec {
    pub path_id: i64,
    pub type_index: i32,
    pub data: Vec<u8>,
}

/// A serialized file of format `version` (17 or 21), little-endian data.
pub fn serialized_file(
    version: u32,
    types: &[TypeSpec],
    objects: &[ObjectSpec],
    externals: &[&str],
    has_type_tree: bool,
) -> Vec<u8> {
    const HEADER: usize = 20;
    let mut meta = W::default();
    meta.cstr("2019.4.9f1");
    meta.i32(19);
    meta.u8(has_type_tree as u8);
    meta.i32(types.len() as i32);
    for t in types {
        meta.i32(t.class_id);
        meta.u8(0); // stripped
        meta.i16(-1); // script type index
        if t.class_id == 114 {
            meta.bytes(&[7; 16]);
        }
        meta.bytes(&[9; 16]); // old type hash
        if has_type_tree {
            meta.bytes(&tree_blob(t.tree.as_ref().expect("a tree"), version));
            if version >= 21 {
                meta.i32(0); // type dependencies
            }
        }
    }
    meta.i32(objects.len() as i32);
    let mut data = Vec::new();
    for o in objects {
        while data.len() % 8 != 0 {
            data.push(0);
        }
        // each record is aligned to 4 relative to the start of the file
        while (HEADER + meta.0.len()) % 4 != 0 {
            meta.u8(0);
        }
        meta.i64(o.path_id)
            .u32(data.len() as u32)
            .u32(o.data.len() as u32)
            .i32(o.type_index);
        data.extend_from_slice(&o.data);
    }
    meta.i32(0); // script types
    meta.i32(externals.len() as i32);
    for e in externals {
        meta.cstr("").bytes(&[0; 16]).i32(0).cstr(e);
    }
    if version >= 20 {
        meta.i32(0); // ref types
    }
    meta.cstr(""); // user information

    let data_offset = (HEADER + meta.0.len() + 15) / 16 * 16;
    let mut f = W::default();
    f.be_u32(meta.0.len() as u32)
        .be_u32((data_offset + data.len()) as u32)
        .be_u32(version)
        .be_u32(data_offset as u32);
    f.u8(0).bytes(&[0; 3]); // little-endian, reserved
    f.bytes(&meta.0);
    while f.0.len() < data_offset {
        f.u8(0);
    }
    f.bytes(&data);
    f.0
}

// ---------------------------------------------------------------------------------------------
// LZ4

/// A literal-only LZ4 block (valid: the last sequence has no match).
pub fn lz4_literals(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let n = data.len();
    if n < 15 {
        out.push((n as u8) << 4);
    } else {
        out.push(0xF0);
        let mut rest = n - 15;
        while rest >= 255 {
            out.push(255);
            rest -= 255;
        }
        out.push(rest as u8);
    }
    out.extend_from_slice(data);
    out
}

/// A greedy LZ4 compressor that finds matches of 4 bytes or more, following the end-of-block
/// rules of the format (the last match starts at least 12 bytes before the end, and the last 5
/// bytes are literals).
pub fn lz4_compress(data: &[u8]) -> Vec<u8> {
    use std::collections::HashMap;
    let mut out = Vec::new();
    let mut table: HashMap<[u8; 4], usize> = HashMap::new();
    let n = data.len();
    let mut anchor = 0usize;
    let mut i = 0usize;
    let limit = n.saturating_sub(12);
    while i < limit {
        let key = [data[i], data[i + 1], data[i + 2], data[i + 3]];
        let candidate = table.insert(key, i);
        if let Some(c) = candidate {
            if i - c <= 65535 {
                let mut len = 4;
                while i + len < n - 5 && data[c + len] == data[i + len] {
                    len += 1;
                }
                emit(&mut out, &data[anchor..i], Some((i - c, len)));
                i += len;
                anchor = i;
                continue;
            }
        }
        i += 1;
    }
    emit(&mut out, &data[anchor..], None);
    out
}

fn emit(out: &mut Vec<u8>, literals: &[u8], m: Option<(usize, usize)>) {
    let lit = literals.len();
    let mlen = m.map(|(_, l)| l - 4);
    let token_lit = if lit >= 15 { 15 } else { lit } as u8;
    let token_match = match mlen {
        Some(l) if l >= 15 => 15,
        Some(l) => l as u8,
        None => 0,
    };
    out.push(token_lit << 4 | token_match);
    if lit >= 15 {
        let mut rest = lit - 15;
        while rest >= 255 {
            out.push(255);
            rest -= 255;
        }
        out.push(rest as u8);
    }
    out.extend_from_slice(literals);
    if let Some((offset, _)) = m {
        out.extend_from_slice(&(offset as u16).to_le_bytes());
        let l = mlen.unwrap();
        if l >= 15 {
            let mut rest = l - 15;
            while rest >= 255 {
                out.push(255);
                rest -= 255;
            }
            out.push(rest as u8);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Bundle

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pack {
    Stored,
    Lz4,
    Lz4Literals,
}

pub struct EntrySpec {
    pub path: &'static str,
    pub flags: u32,
    /// Offset and size in the decompressed stream
    pub offset: u64,
    pub size: u64,
}

/// A UnityFS version 6 bundle: the blocks hold the given contents, packed as stated.
pub fn bundle(blocks: &[(Vec<u8>, Pack)], entries: &[EntrySpec]) -> Vec<u8> {
    let (info, body) = bundle_parts(blocks, entries);
    assemble_bundle(&info, &body)
}

/// The block info (not yet compressed) and the packed blocks of a bundle.
pub fn bundle_parts(blocks: &[(Vec<u8>, Pack)], entries: &[EntrySpec]) -> (Vec<u8>, Vec<u8>) {
    let mut packed: Vec<(u32, u32, u16, Vec<u8>)> = Vec::new();
    for (content, how) in blocks {
        let (flags, data) = match how {
            Pack::Stored => (0u16, content.clone()),
            Pack::Lz4 => (3u16, lz4_compress(content)),
            Pack::Lz4Literals => (2u16, lz4_literals(content)),
        };
        packed.push((content.len() as u32, data.len() as u32, flags, data));
    }
    let mut info = W::default();
    info.bytes(&[0; 16]);
    info.be_i32(packed.len() as i32);
    for (u, c, f, _) in &packed {
        info.be_u32(*u).be_u32(*c).be_u16(*f);
    }
    info.be_i32(entries.len() as i32);
    for e in entries {
        info.be_i64(e.offset as i64)
            .be_i64(e.size as i64)
            .be_u32(e.flags)
            .cstr(e.path);
    }
    let body = packed.iter().flat_map(|p| p.3.iter().copied()).collect();
    (info.0, body)
}

/// Puts a block info (compressed here with LZ4) and the packed blocks behind a header.
pub fn assemble_bundle(info: &[u8], body: &[u8]) -> Vec<u8> {
    let info_packed = lz4_compress(info);
    let mut f = W::default();
    f.cstr("UnityFS");
    f.be_u32(6);
    f.cstr("5.x.x");
    f.cstr("2019.4.9f1");
    let total = f.0.len() + 8 + 4 + 4 + 4 + info_packed.len() + body.len();
    f.be_i64(total as i64);
    f.be_u32(info_packed.len() as u32);
    f.be_u32(info.len() as u32);
    f.be_u32(0x43);
    f.bytes(&info_packed);
    f.bytes(body);
    f.0
}

/// A deterministic byte stream with structure that LZ4 can find (and some that it cannot).
pub fn sample_bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        match x % 5 {
            0 => out.extend(std::iter::repeat((x >> 8) as u8).take((x >> 16) as usize % 40)),
            1 | 2 => out.extend_from_slice(b"the quick brown fox jumps over the lazy dog "),
            _ => out.extend_from_slice(&x.to_le_bytes()),
        }
    }
    out.truncate(n);
    out
}
