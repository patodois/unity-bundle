# unity-bundle

Reads the asset bundles and serialized files of **Unity 2018.4 and 2019.4** and turns the objects in
them into Rust values with [serde](https://serde.rs), through the type trees the files carry.

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
  that the data cannot back up (see [Hostile input](#hostile-input)).

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

    for info in file.objects.iter().filter(|o| o.class_id.0 == 28) {
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

`Option<T>` is always `Some`, enums and byte buffers (`deserialize_bytes`) are refused, and padding
follows the `align` flag of each node as Unity writes it.

Minimum supported Rust version: 1.70.

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
remain. Reads of an object stop at the object's end. What is allocated is proportional to the input, never to a number
in it: a decompressed block is at most 255 times its stored bytes, and the parsed tables cost at most about 10 times
the bytes they were read from (a type record of 23 bytes becomes a 240-byte struct). Tests (`tests/hostile.rs`) run, with an
allocator that records the largest single allocation, 30,000 mutated bundles, 60,000 mutated block
tables, 60,000 mutated serialized files and 100,000 mutated objects, every truncation and every
single-byte change of the samples, and a set of decompression, type tree, count and
size bombs; none panics, and none allocates more than 8 MiB at once. The oracle repeats this on mutated
copies of the game's bundles (`oracle GAME --fuzz N`): 120,000 mutated bundles and 119,600 mutated serialized
files, no panic, the largest single allocation 13 MB (decompressing a 12 MB bundle).

## Provenance

This crate was written by an AI agent (Claude), at the direction of the owner of
[patodois](https://github.com/patodois), and what it read to learn the formats is stated here.

- **Read in full or in the parts that matter**: the source of
  [rabex](https://github.com/jakobhellermann/RustyAssetBundleEXtractor) (Rudolf Kolbe and Jakob
  Hellermann, MIT OR Apache-2.0), at the fork revision that nocturne-unpack used: the bundle
  header, block and directory reading, the serialized file reader, the type tree blob reader, the
  serde deserializer, and the first entries of its common-strings and class-id tables. That is where
  the layouts were learned and where the deserializer's behaviour, which the oracle compares
  against, was understood (which node types serve which serde types, the map as a sequence of pairs,
  the lenient map keys). No code was copied, and the implementation here is organised differently
  (slices and a cursor instead of `Read + Seek`, one `Kind` per node, a skip function instead of
  `IgnoredAny` through `deserialize_any`, checked sizes), but it is not a clean-room
  implementation: the author had read rabex first.
- **Known from general knowledge, not consulted in this work**: AssetStudio (MIT) and UnityPy (MIT),
  from which the common-strings table's contents and the class ids' names are remembered, and which
  document the same formats. The common-strings offsets were checked against the game's files
  (below), the class names against the root types of the game's type trees.
- **Public format descriptions cited**: the
  [LZ4 block format](https://github.com/lz4/lz4/blob/dev/doc/lz4_Block_format.md) (the decoder is
  written from it) and Unity's
  [class ID reference](https://docs.unity3d.com/Manual/ClassIDReference.html).
- **Evidence**: the game's own files. `docs/FORMATS.md` states each fact with the numbers
  from the 3,088 bundles, and a second, independent reading (a Python script written for the
  hexdumps in that file) agreed with this crate's on the example it shows.
- **Used only to compare, never linked**: rabex (the oracle).

## Licence

MIT OR Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
