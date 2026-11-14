# unity-bundle

unity-bundle reads **Unity 2018.4.13 and 2019.4.9 to 2019.4.10** asset bundles and serialized files for Rust developers, turning type-tree objects into [serde](https://serde.rs) values within the checked [scope](#scope).

- **UnityFS bundles** (container format version 6): the header, the block table, the file directory;
  LZ4 and LZ4HC blocks and stored blocks; whole files or a byte range of a file (only the blocks
  that cover the range are decompressed, which is what makes a few kilobytes out of a 400 MB `.resS`
  cheap).
- **Serialized files** (format versions 17 and 21, little-endian): the type table with its type
  trees, the objects, the files they point into.
- **Objects**: a `serde` deserializer that walks the type tree. A Rust type names the fields it wants
  and the rest is skipped, so no class layout is written down in code and a field Unity adds or moves
  does not break a reader that does not use it.
- Reads from slices (a memory map works: anything `AsRef<[u8]>`), has one dependency (`serde`), and
  forbids `unsafe`.
- Malformed input gives an `Error`, never a panic, and no size in the data decides an allocation
  that the data cannot back up; [Hostile input](#hostile-input) states what the data can back.

It reads what a game of that vintage ships, and nothing else: no writing, no LZMA, no encrypted
bundles, no other format versions (they are refused with `Error::Unsupported` and a reason), no big-endian
files, no managed references. It was written for
[nocturne-unpack](https://github.com/patodois/nocturne-unpack), which reads Shin Megami Tensei III
Nocturne HD Remaster, and checked on every bundle of that game (see [Verification](#verification)).
The formats, with the evidence from those files, are in [docs/FORMATS.md](docs/FORMATS.md), and what
it took over from rabex, call by call, in [docs/SUBSET.md](docs/SUBSET.md).

```rust,no_run
use serde::Deserialize;
use unity_bundle::{Bundle, SerializedFile};

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Texture2D {
    m_Name: String,
    m_Width: i32,
    m_Height: i32,
    m_TextureFormat: i32,
    // streamed textures keep their pixels in a .resS file of the bundle
    m_StreamData: StreamingInfo,
}

#[derive(Deserialize)]
struct StreamingInfo {
    offset: u32,
    size: u32,
    path: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("dvl0x01")?;
    let bundle = Bundle::new(&bytes[..])?;

    // The serialized file with the objects
    let entry = bundle.main_serialized_entry().ok_or("no serialized file")?;
    let data = bundle.read_entry(entry)?;
    let file = SerializedFile::parse(&data)?;

    for info in file.objects().iter().filter(|o| o.class_id.0 == 28) {
        let t: Texture2D = file.read(&data, info)?;
        // Only the compression blocks that cover the range are decompressed
        let name = t.m_StreamData.path.rsplit('/').next().unwrap();
        let res = bundle.entry(name).ok_or("no resS")?;
        let pixels = bundle.read_entry_range(res, t.m_StreamData.offset as u64, t.m_StreamData.size as usize)?;
        println!("{} {}x{} format {} {} bytes", t.m_Name, t.m_Width, t.m_Height, t.m_TextureFormat, pixels.len());
    }
    Ok(())
}
```

`cargo run --release --example inspect -- FILE [--tree CLASS_ID] [--hex N]` prints what a bundle holds.

## How type trees map to serde types

| type tree node | serde |
|---|---|
| `bool`, `UInt8`, `SInt8`, `UInt16`/`unsigned short`, `SInt16`/`short`, `UInt32`/`unsigned int`/`Type*`, `SInt32`/`int`, `UInt64`/`unsigned long long`/`FileSize`, `SInt64`/`long long`, `float`, `double`, `char` | the primitive of that type and no other (`float` is not read as `u32`) |
| `string` | `String` (a visitor that wants bytes gets them when they are not UTF-8) |
| `vector`, `set`, ... (one `Array` child) | a sequence |
| `map` | a sequence of `(key, value)` tuples, or a map when the key is a string or a `UInt32` (as decimal text) |
| `TypelessData` | a sequence of `u8` |
| anything else with children | a struct (fields by name, in any order, the rest skipped) or a tuple (children in order) |
| a field the Rust type names but the tree does not have | `#[serde(default)]`, or an error naming it |

`Option<T>` is always `Some`, enums and byte buffers (`deserialize_bytes`) are refused, strings are always
padded to 4 bytes, and the other padding follows the `align` flag of each node as Unity writes it.

Limits of the mapping: fields are skipped before `serde` can see them, so `#[serde(deny_unknown_fields)]` never
fires; `#[serde(flatten)]` and a `HashMap` read from a struct node fail (a struct node is not a `map` node, only
a `map` node is); `()` reads nothing; a tuple takes the node's children in order. Each is a test in
`tests/de.rs`. `ClassId::name()` is `None` for an id that is not in the crate's table, which holds only the
names found in both rabex's table and AssetStudio's (and 94): an id Unity has can be missing, and so can one a
newer Unity added.

## Scope

Exactly what the one game it was written for ships: Unity **2018.4.13** and **2019.4.9 to 2019.4.10**, UnityFS
format 6, serialized file formats 17 and 21. Refused with an error that says why, because no file of the game has
them and nothing could be checked against them:

- zero padding before the block info (the header's fields are followed directly by it), and the header flag
  `0x200` (alignment of the block info) or any flag beyond the compression and `0x40`;
- LZMA and LZHAM blocks, block info at the end of the file, encrypted bundles, other signatures, format 7;
- serialized formats 18 to 20 and 22 and later, big-endian files, managed references;
- common-string offsets above 1051 (`Type*`, the last entry the game's trees reach): both public tables go on
  with names that later Unity versions added, and those could be added with a citation, but a 2019.4 tree
  stores such names itself, so an offset there would be a sign of something else.

Minimum supported Rust version: 1.71 (the newest compatible `syn` needs it).

## Verification

The crate was compared with [rabex](https://github.com/jakobhellermann/RustyAssetBundleEXtractor)
(our fork at revision `56dd8a4`, the one nocturne-unpack used until now) as a black box on every file of
the installed game, by the oracle in `tools/oracle` (it needs the game and the fork, so it is not part
of the crate or of CI). `tools/oracle/REPORT.txt` is its output. On the 3,088 bundle files (2,251 base
game, 837 DLC, 12.1 GB on disk, 18.9 GB decompressed) and 9 loose serialized files:

- the headers, the 146,779 blocks and the directories: identical;
- every decompressed byte of every bundle, and every byte range read from a file: identical;
- the serialized files' tables and **all 16,822 type trees (1.87 million nodes)**: identical;
- a digest of **all 1,468,692 objects that have a type tree**, read through `deserialize_any` with both
  libraries: identical, and none fails in either;
- the typed values of the 14 classes nocturne-unpack reads (1,282,304 objects, including all 15,930
  textures and all 112,166 meshes) and the 15,753 streamed `.resS` ranges they name: identical.

No difference in any value. The differences in behaviour that exist (one of them a padding rabex does
not do, which no value shows) are listed in
[docs/FORMATS.md](docs/FORMATS.md#differences-from-rabex), along with how sensitive the comparison is: of three
deliberate faults put into the crate one at a time, two are caught and the third (the padding that rabex
does not do) cannot be seen through values.
`tools/survey.py` is a second, independent reading of the container (its own LZ4 decoder, no code shared).

## Hostile input

Every size and count read from the data is checked against what the data can hold before anything
is allocated: a block's decompressed size against LZ4's best ratio for its stored size, an array's
element count against the bytes that remain, a type tree's node count against the bytes that
remain. What a checked size asks for is then allocated in a way that can fail: when the memory cannot be had
the result is `Error::OutOfMemory`, not an abort. Reads of an object stop at the object's end. Two budgets stop
sizes that pass those checks from multiplying: the strings copied for a type tree (64 bytes per node plus the
tree's string buffer, because many nodes can name one string) and the values visited while reading an object
(16 per byte plus 1,024, because elements that take no bytes cost nothing to the data).

Each step is bounded by a constant multiple of the bytes it reads (the figures are for 64-bit targets):

| step | the most it holds |
|---|---|
| one LZ4 block, or the block info | 255 times its stored bytes, and 64 MiB |
| opening a bundle | 2.8 times the decompressed block info: 12 bytes of block table and 16 of block offsets for a 10-byte record (the block info is freed before the offsets are built); 48 bytes and the name for a file entry of 21 bytes or more |
| the type table | 240 bytes for a type record of 23 bytes, 10.4 times |
| a type tree | 104 bytes for a node record of 24 bytes (format 17) or 32 (format 21), plus its strings, which are at most 64 bytes per node and the tree's string buffer: up to 7 times, and 112 bytes per node more while it is built (up to 12 times) |
| the object table | 32 bytes and at most 39 of index for a record of 20 bytes, under 4 times |
| script types, externals | 16 bytes for 12, and 72 bytes and the path for 22 or more: under 4 times |
| the values of an object | 16 values per byte of the object, plus 1,024; what a value costs is up to the Rust type that receives it |

The steps multiply with each other, and the crate does not cap the product. The worst cases:

- **Opening a bundle.** 263 KB of stored block info can decompress to 64 MiB of block records (empty blocks are
  valid), whose tables then take 188 MB for as long as the `Bundle` lives. Opening a bundle never holds more than
  about that, whatever the file says.
- **A serialized file out of a bundle.** Its bytes can be 255 times what the bundle stores, and a type table
  10.4 times those: 2,660 bytes of table for a stored byte. A bundle of 1 MiB can hold a serialized file of
  255 MiB of type records, which parse to 2.6 GiB.
- **A generic value.** The visit budget counts values, not their size. A struct that names its fields holds only
  those, but a type that keeps everything it is shown holds every value visited. Read as `serde_json::Value`, an
  object whose tree is an array of arrays of elements that take no bytes came to 0.7 KB per byte of the object
  when the elements are empty structs and to 5.4 KB when they are structs with one empty field (measured), and it
  is more when the field names are long, since such a value copies the name of each field it visits. Through LZ4
  that is again times 255 per stored byte.

A caller that reads files it does not trust and needs a lower ceiling has each number before the memory is asked
for: `Entry::size` before `read_entry`, the length of the data before `SerializedFile::parse`, `ObjectInfo::size`
before `read`.

Tests (`tests/hostile.rs`) run, with an allocator that tracks the
memory live at once, 30,000 mutated bundles, 60,000 mutated block tables, 60,000 mutated serialized files and
100,000 mutated objects, every truncation and every single-byte change of the samples, and a set of
decompression, type tree, count, string and visit bombs; none panics, and none has more than 8 MiB live at once
while reading these small inputs. The two bombs found by review (a shared string, 763 MiB before the budget; an
array of arrays of empty structs, 346 MiB) are tests that fail without their budget. One test builds the inputs
that cost the most for their size and checks the multiples in the table above, and `tests/out_of_memory.rs`
refuses large allocations and checks that each table and buffer then fails with `Error::OutOfMemory`. The oracle
repeats the mutations on copies of the game's bundles (`oracle GAME --fuzz N`): 120,000 mutated bundles and 119,600
mutated serialized files, no panic, and at most 18.6 MB live at once (reading a mutated 12 MB bundle whole).

## Provenance

The code was written with AI coding agents (Anthropic's Claude and OpenAI's GPT models) under the maintainer's direction.
AI-assisted contributions are welcome when their use is disclosed and the changes are reviewed.
The format sources and technical provenance are stated below.

- **Read: rabex.** The source of [rabex](https://github.com/jakobhellermann/RustyAssetBundleEXtractor)
  (Rudolf Kolbe and Jakob Hellermann, MIT OR Apache-2.0) at the fork revision that nocturne-unpack used,
  `56dd8a4d8d5048b6ee7664eadc89d98343c66002`: the bundle header, block and directory reading, the serialized
  file reader, the type tree blob reader, the serde deserializer, `read_ext.rs`, the Unity version parser, and
  the common-strings and class-id tables. That is where the layouts were learned and where the deserializer's
  behaviour, which the oracle compares against, was understood (which node types serve which serde types, the
  map as a sequence of pairs, the lenient map keys). **The structure follows rabex's.** The `Kind`
  classification of nodes (`typetree.rs`) follows rabex's `TypeTreeNode::classify` arm for arm and `Kind`
  mirrors its `TypetreeNodeKind`; the `deserialize_any` dispatch (`de.rs`) has the same arms in the same order,
  and `KeyDe`, `MapKeyDe` and `ByteSeq` mirror its `FixedKeyDeserializer`, `LenientMapKeyDeserializer` and
  `ByteSeqDeserializer`. No text or expressions were pasted, and some things are done differently (slices
  and a cursor instead of `Read + Seek`, a skip function instead of `IgnoredAny` through `deserialize_any`,
  checked sizes and budgets), but this is not a clean-room implementation: the author had read rabex first and
  its design is in the result. `NOTICE` credits it.
- **Read: AssetStudio** (MIT), revision `d158e864b556b5970709c2a52e47944d53aa98a2`, only these files, fetched
  read-only to check this crate's tables and one claim: `CommonString.cs`, `ClassIDType.cs`, the header and
  directory part of `BundleFile.cs`, and a search of `SerializedFile.cs` for the string table. Nothing was
  copied; the tables were first written from the author's memory and kept only after the entry-by-entry
  comparison in `tools/check_tables.py` (`tools/TABLES.txt`).
- **UnityPy** (MIT): `UnityPy/helpers/Tpk.py` and `TypeTreeNode.py` at revision
  `417d59522a998fdee9a9c2b29fe35f3063222457` were read only to see where its tables come from (a binary TPK
  file). Its tables were **not** read or compared, and nothing else of it was consulted.
- **Not consulted**: Unity's class ID reference and the AssetRipper TPK database (rabex's tables are generated
  from the latter). Unity's documentation is not used as a source for any fact here.
- **Public format description**: the
  [LZ4 block format](https://github.com/lz4/lz4/blob/dev/doc/lz4_Block_format.md) (the decoder is
  written from it).
- **Evidence**: the game's own files. `docs/FORMATS.md` states each fact with the numbers
  from the 3,088 bundles, and a second, independent reading (a Python script written for the
  hexdumps in that file) agreed with this crate's on the example it shows.
- **Used only to compare, never linked**: rabex (the oracle).

## Licence

MIT OR Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
