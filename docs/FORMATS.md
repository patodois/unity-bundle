# Formats

What `unity-bundle` reads, written as facts. Each fact says where it comes from:

- **game**: seen in the files of Shin Megami Tensei III Nocturne HD Remaster (Steam, Windows build): 2,251 bundles
  in `smt3hd_Data/StreamingAssets/PC`, 837 in `dlc/*` (3,088 files, 12.1 GB), and 9 loose serialized files
  in `smt3hd_Data`. The numbers come from the oracle run in `tools/oracle/REPORT.txt` (this crate against
  rabex, on every file) and from `tools/survey.py` (`tools/SURVEY.txt`), an independent script in Python with its own
  LZ4 decoder, sharing no code with the crate, that read the header, the block info and the first block of every
  bundle by seeking.
- **spec**: a public description, cited.
- **choice**: a decision of this crate, with its reason.

All numbers in the bundle header and block info are **big-endian**; in a serialized file the first 20
bytes are big-endian and everything after them is **little-endian**.

The examples are from `dvl0x01`, a 2.7 MB demon bundle written by Unity 2019.4.9f1; `cargo run --example
inspect -- FILE --hex 112 --tree 28` prints them.

## 1. The bundle (UnityFS, container format version 6)

### 1.1 Header

```
00000000  55 6e 69 74 79 46 53 00 00 00 00 06 35 2e 78 2e  UnityFS.....5.x.
00000010  78 00 32 30 31 39 2e 34 2e 39 66 31 00 00 00 00  x.2019.4.9f1....
00000020  00 00 2a 21 71 00 00 01 2a 00 00 02 57 00 00 00  ..*!q...*...W...
00000030  43 1e 00 01 00 30 27 00 02 07 00 41 8f 18 00 03  C....0'....A....
```

| field | type | in the example |
|---|---|---|
| signature | NUL-terminated string | `UnityFS` |
| format version | u32 | `6` |
| player version | NUL-terminated string | `5.x.x` |
| Unity version | NUL-terminated string | `2019.4.9f1` |
| size | i64 | `2761073`, the size of the file |
| block info, compressed size | u32 | `0x12a` = 298 |
| block info, decompressed size | u32 | `0x257` = 599 |
| flags | u32 | `0x43` |

The block info starts at the next byte (offset 49 here, the `1e` at `0x31`), with no padding.

**game**: in all 3,088 files the signature is `UnityFS`, the format version 6, the player version `5.x.x`,
the flags `0x43`, the size field equals the size of the file, and the block info follows the header's
fields directly. The Unity versions are `2019.4.10f1` (2,337 files), `2019.4.9f1` (722) and
`2018.4.13f1` (29; see 2.1).

The low 6 bits of the flags are the compression of the block info (`3` = LZ4HC); bit 6 (`0x40`) says
that the block info holds the directory too, which is the only layout in version 6; bit 7 (`0x80`, block
info at the end of the file) and every bit above it are clear in all files and are refused.

**choice**: Other readers (AssetStudio, and the rabex fork this crate was compared with) accept zero
padding to a multiple of 16 before the block info in version 6 bundles of 2019.4. No file of the game
has any, so it is not read: such a bundle fails with a decompression error rather than being guessed at.

### 1.2 Block info

LZ4-decompressed, 599 bytes in the example:

```
00000000  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  ................
00000010  00 00 00 27 00 02 00 00 00 00 8f 18 00 03 00 02  ...'............
00000020  00 00 00 01 23 19 00 03 00 02 00 00 00 00 73 03  ....#.........s.
```

| field | type | in the example |
|---|---|---|
| a 16-byte hash | 16 bytes | all zero (not read; zero in all 3,088 files) |
| block count | i32 | `0x27` = 39 |
| per block: decompressed size | u32 | `0x20000` = 131072 |
| per block: compressed size | u32 | `0x8f18` = 36632 |
| per block: flags | u16 | `3` |
| file count | i32 | 3 |
| per file: offset | i64 | 0 |
| per file: size | i64 | 4080836 |
| per file: flags | u32 | `4` |
| per file: path | NUL-terminated string | `CAB-802f5b94c795233d51dd15705e2fab37` |

**game**: the block info is used to its last byte in all 3,088 files. The files hold 146,779 blocks:
36,241 stored (flags `0x0`, 4.7 GB of the 18.9 GB decompressed) and 110,538 LZ4HC (flags `0x3`); no other
value, and no flag bit beyond the compression. A block is 131,072 bytes decompressed (`0x20000`) except the
last of a stream, and none is larger.

### 1.3 Blocks and files

The blocks follow the block info, back to back; their decompressed contents, concatenated, are the
*stream*. A file is a range of the stream (`offset`, `size`). Blocks start and end wherever they like, not at
file boundaries.

**game**: in all 3,088 files the blocks end exactly at the end of the file; the files are listed in stream
order, start at 0, are contiguous and cover the whole stream; the first one is the serialized file. There are
4,288 files: 3,088 serialized files (flags `0x4`, named `CAB-` and 32 hex digits, one per bundle) and 1,200 raw
ones (flags `0x0`) named like it plus `.resS` (665; streamed texture and mesh data) or `.resource` (535;
streamed audio).

Reading a range of a file decompresses the blocks that cover it, looked up by the stream offset each one
starts at, and copies the part asked for. A stored block (flags `0`) has equal compressed and decompressed
sizes.

### 1.4 LZ4

A block is one LZ4 *block* (not a frame): sequences of `token, [literal length extension], literals, offset
(u16 LE), [match length extension]`, the last sequence holding only literals. The format is described in the
LZ4 project's [block format](https://github.com/lz4/lz4/blob/dev/doc/lz4_Block_format.md) (**spec**). LZ4HC is
the same format; it differs only in how hard the encoder looked for matches. **game**: every compressed block
has flag 3 (LZ4HC); none has flag 2 (LZ4). Both are decoded by the same code, so flag 2 is exercised by the
tests with synthetic blocks only.

**choice**: The decoder is told the decompressed size, creates the output at that size, and fails if the
stream produces more or fewer bytes. Before that it refuses a block whose decompressed size is more than 255
times its compressed size (no LZ4 stream expands more: each byte of a match length extension adds 255 bytes)
or more than 64 MiB, so a table cannot make the decoder allocate what the stored bytes could not fill. The
decoder copies 16 bytes at a time where it can (writing into output that is not yet needed), which makes it
about 2.9 GB/s against 3.7 GB/s for the `lz4_flex` crate that rabex uses, in safe code.

### 1.5 What is not read

LZMA (flag 1) and LZHAM (flag 4) blocks, block info at the end of the file, encrypted bundles (`0x400`), the
other signatures (`UnityWeb`, `UnityRaw`, `UnityArchive`) and format version 7. None is in the game's files,
so nothing could be checked against them; they are refused with `Error::Unsupported`.

## 2. The serialized file

The main file of a bundle (the first `0x4` entry that does not end in `.sharedAssets`) and the loose files in
`smt3hd_Data` (`level0`, `sharedassets0.assets`, `globalgamemanagers`, ...).

### 2.1 Versions

| format | written by | files in the game |
|---|---|---|
| 21 | Unity 2019.4 | 3,059 bundles (2,227 base game, 832 DLC) and the 9 loose files |
| 17 | Unity 2018.4 | 29 bundles (24 base, 5 DLC), named `e001`, `e001m`, `e002`, ... `e550` |

Both are read; no other. They differ in three places: from format 19, a type tree node record is 32 bytes and
not 24 (a `u64` hash of the referenced type is added); from 20, a list of reference types follows the
externals; from 21, each type carries a list of type dependencies (or, for a reference type, three names).
Formats 18 to 20 and 22 and later are refused, since nothing in the game could show what they look like.
**game**: every serialized file has little-endian data (the byte after the four sizes is 0) and three zero
reserved bytes after it.

### 2.2 Header (20 bytes, big-endian)

```
00000000  00 02 45 e4 00 3e 44 c4 00 00 00 15 00 02 46 00  ..E..>D.........
00000010  00 00 00 00 32 30 31 39 2e 34 2e 39 66 31 00 13  ....2019.4.9f1..
```

| field | type | in the example |
|---|---|---|
| metadata size | u32 | `0x245e4` = 148964 |
| file size | u32 | `0x3e44c4` = 4080836 |
| format version | u32 | `0x15` = 21 |
| data offset | u32 | `0x24600` = 148992 |
| endianness | u8 | `0` (little) |
| reserved | 3 bytes | zero |

**game**: the file size is the size of the bundle entry in all 3,088 bundles. The data offset is a multiple of
16 in all 3,097 files and is where the objects' data begins; in 2,854 files it is the header plus the metadata
rounded up to 16, in 243 it is larger (up to 4096), so it is stored, not derived. The metadata size is not
used by this crate.

### 2.3 Metadata (little-endian)

In order from byte 20: the Unity version (NUL-terminated string, `2019.4.9f1`), the target platform (i32;
`19`, Windows standalone 64-bit, in every file), the type tree flag (u8; `1` in all bundles, `0` in the loose
files, see 2.7), the type table (i32 count, then the types), the object table (i32 count, then the objects),
the script types (i32 count, then the entries), the externals (i32 count, then the entries), from format 20 the
reference types (i32 count, then types), and a user information string.

**game**: the user information is empty in all files, there are no reference types, and no type is stripped.

### 2.4 Types

An entry of the type table, which objects refer to by index:

| field | type | |
|---|---|---|
| class id | i32 | see section 3 |
| stripped | u8 | 0 in all files |
| script type index | i16 | -1 except for scripts |
| script hash | 16 bytes | only when the class id is 114 (`MonoBehaviour`) |
| type hash | 16 bytes | |
| type tree | blob | only when the type tree flag is set; see 2.5 |
| type dependencies | i32 count, then i32s | format 21 and the flag set; the count is 0 in all files |

**game**: 17,001 types in 3,097 files. In the bundles every type has a type tree; in the 9 loose files none has
(179 types). The 2,492 `MonoBehaviour` types of the bundles (and 80 of the loose files) have a script hash and a
script type index other than -1.

### 2.5 Type trees

A tree is a flat list of nodes in pre-order, each with its depth:

| field | type | |
|---|---|---|
| node count | i32 | |
| string buffer size | i32 | |
| nodes | count × 24 (format 17) or 32 (format 21) bytes | |
| string buffer | bytes | |

A node record: `version` (u16), `level` (u8; 0 for the root), `type flags` (u8; bit 0 marks arrays), `type
string offset` (u32), `name string offset` (u32), `byte size` (i32; -1 when it varies), `index` (i32; the
node's position in the pre-order), `meta flags` (i32), and from format 19 the `reference type hash` (u64; zero
except for managed references).

A string offset with the top bit clear is a position in the tree's own string buffer (a NUL-terminated
string); with the top bit set, the low 31 bits are a position in the **common strings** table, a buffer of
NUL-terminated names that Unity keeps in the player (`AABB`, `Array`, `Texture2D`, `m_Name`, ...). The table is
in `src/common_strings.rs`, with each entry's offset.

**game**: 1,872,412 nodes in 16,822 trees; `index` is the pre-order position in all of them, the root is at
level 0 and a level is never more than one deeper than the one before.

*Common strings.* The game's trees use 64 of the 101 entries of the table (the instrumented run is in
`REPORT.txt`): `AABB`, `AnimationClip`, `AnimationCurve`, `Array`, `Base`, `BitField`, `bool`, `char`,
`ColorRGBA`, `data`, `double`, `first`, `float`, `GameObject`, `GUID`, `int`, `map`, `Matrix4x4f`,
`MonoBehaviour`, `MonoScript`, `m_Curve`, `m_Enabled`, `m_GameObject`, `m_Index`, `m_Name`, `m_Script`, `m_Type`,
`m_Version`, `pair`, `PPtr<Component>`, `PPtr<GameObject>`, `PPtr<Material>`, `PPtr<MonoBehaviour>`,
`PPtr<MonoScript>`, `PPtr<Object>`, `PPtr<Sprite>`, `PPtr<Texture>`, `PPtr<Texture2D>`, `PPtr<Transform>`,
`Quaternionf`, `Rectf`, `RectOffset`, `second`, `set`, `size`, `SInt16`, `SInt64`, `SInt8`, `staticvector`,
`string`, `TextAsset`, `Texture2D`, `Transform`, `TypelessData`, `UInt16`, `UInt64`, `UInt8`, `unsigned int`,
`vector`, `Vector2f`, `Vector3f`, `Vector4f`, `Gradient`, `Type*`. Every one decodes to the same string in this
crate and in rabex. The other 37 entries (for example `SInt32`, `UInt32`, `m_MetaFlag`, `m_IsArray`) are not used
by any tree of the game, so their text is from the public tables of AssetStudio and UnityPy and was not checked
against the game; the offsets of the entries after them are checked, because the later ones are used. The table
stops at `Type*` (offset 1051): names that later Unity versions added at the end of it (`int2_storage`,
`m_PrefabAsset`, ...) are stored in each tree's own string buffer by 2019.4, so an offset past `Type*` is refused.

*Meta flags.* Only bit 14 (`0x4000`, *align bytes*) is used by this crate: after the value of a node that has
it, the reader skips to the next multiple of 4, counted from the start of the object's data. **game**:

| node | count | where the flag is |
|---|---|---|
| `string` | 98,212 | on its `Array` child in all (97,429), and on the `string` node as well in 783 |
| `TypelessData` | 1,359 | on the node |
| `vector`, `set`, `staticvector`, ... | 115,403 | on the `Array` child only in 75,938, on the node only in 547, on both in 3,826, on neither in 35,092 |
| `map` | 11,859 | on none of the `map`, its `Array` or its `pair` |
| other nodes with children | 308,586 | on 1,741 |
| `bool` 21,043, `SInt8` 17,058, `UInt8` 12,739, `SInt16` 964, `UInt16` 302, `int` 1,310, `float` 572 | | all with the flag, in those numbers |

### 2.6 The object table

For each object, a record that starts at a multiple of 4 (counted from the start of the file):

| field | type | |
|---|---|---|
| path id | i64 | the object's id inside the file |
| offset | u32 | from the data offset |
| size | u32 | |
| type index | i32 | into the type table; gives the class id |

**game**: 1,478,885 objects (1,468,692 in the bundles, 10,193 in the loose files). In every file the path ids
are unique, the objects are disjoint and their offsets are multiples of 8 from the data offset. When a path id
is repeated (Unity does not write that), the last object with it is the one found.

An object's data is `size` bytes at `data offset + offset`. This crate reads an object from exactly those
bytes: a value that runs past them is an error.

### 2.7 Files without type trees

**game**: the loose files in `smt3hd_Data` (the player's own data) have the type tree flag `0`: their types
carry no trees, so their objects can be listed and their bytes read, but not deserialized (`Error::NoTypeTree`).

### 2.8 Externals and script types

An external (`m_FileID` 1, 2, ... of a `PPtr`): an empty NUL-terminated string, a 16-byte GUID, an i32 kind, and
a NUL-terminated path such as `archive:/CAB-16e852a5.../CAB-16e852a5...` (the serialized file of another
bundle) or `Resources/unity_builtin_extra`. **game**: 1,207 externals; the string is empty and the kind 0 in
all; 373 have a GUID other than zero.

A script type: an i32 file index, padding to 4, an i64 path id. **game**: 2,492 in the bundles and 80 in the loose
files, one per `MonoBehaviour` type; they are listed, not otherwise used by this crate.

## 3. Class ids

The number Unity gives each kind of object. `src/class_id.rs` has 236 names from Unity's
[class ID reference](https://docs.unity3d.com/Manual/ClassIDReference.html) (**spec**). **game**: the objects of
the bundles belong to 41 classes (ids 1, 4, 20, 21, 23, 28, 33, 43, 48, 49, 58, 61, 65, 74, 81, 83, 84, 89, 90,
91, 95, 108, 111, 114, 115, 120, 136, 137, 142, 198, 199, 200, 213, 222, 223, 224, 225, 290, 329 and the two
large ids 1183024399, `LookAtConstraint`, and 1773428102, `ParentConstraint`). For each class with a type tree,
the root of the tree is named like the class (`Texture2D`, `SkinnedMeshRenderer`, ...), except for
`MonoBehaviour` (114), whose roots carry the C# class name; and rabex's (much larger) table gives the same name
for every id the game uses. Three names are not confirmed by the game's trees, because the ids appear only
in the loose files, which have none: 94 `ShaderNameRegistry` (older manuals say `ScriptMapper`), 937362698
`VFXManager` and 1403656975 `StreamingManager`; these three are rabex's.

## 4. Reading an object through its type tree

The data of an object is the values of the children of the tree's root, in order, each read as its node says.

| node `type` | what is read |
|---|---|
| `bool`, `UInt8`, `SInt8`, `char` | 1 byte |
| `UInt16`, `unsigned short`, `SInt16`, `short` | 2 bytes |
| `UInt32`, `unsigned int`, `Type*`, `SInt32`, `int`, `float` | 4 bytes |
| `UInt64`, `unsigned long long`, `FileSize`, `SInt64`, `long long`, `double` | 8 bytes |
| `string` | u32 length, that many bytes |
| `TypelessData` | u32 length, that many bytes |
| `Array`, and a node whose only child is an `Array` (`vector`, `set`, `staticvector`, ...) | u32 count, then that many values of the type of the `Array`'s second child |
| `map` | an array whose elements are `pair`s: u32 count, then the `first` and the `second` value of each |
| `Type` | nothing |
| any other node with children | its children, in order |
| any other node | nothing |
| `ReferencedObject`, `ReferencedObjectData`, `ManagedReferencesRegistry` | not read (an error) |

After a node whose `meta flags` have bit 14, padding to a multiple of 4. A `string` is padded when the `string`
node or its `Array` child has the bit, a `TypelessData` when the node (or its first child) has it, a `vector`
or `map` when it or its `Array` child has it.

**choice** (counts bounded by the data): an array's, map's, string's or typeless data's count is refused when it
exceeds the bytes that remain in the object divided by the least size of an element (at least 1, even for
types that take no bytes). A tree 255 levels deep (the most a node record can state) is read without
recursion beyond its depth.

## Differences from rabex

Found by the oracle or by reading, and why. In the game's files none of them changes a value.

1. **Padding after `TypelessData`.** rabex does not pad after it; this crate pads when the node says so, as for
   every other node and as Unity's serializer writes it. The game has 19 textures whose inline `image data` has
   a length that is not a multiple of 4 (RGB24 textures, format 3, in `f014_408`, `f015_408`, `en_base_09`,
   `f014_602`, ... and two mip levels in `f615`); in each, the field after the image data is `m_StreamData`,
   which is all zero, so reading it 1 to 3 bytes early, as rabex does, gives the same values. The two readers
   disagree about where the object's remaining bytes are but not about any value. The unit tests cover the case
   with data in the field after.
2. **Padding after strings.** rabex always pads; this crate pads when the `string` node or its `Array` child
   has the flag, which every string of the game does.
3. **Padding after maps.** rabex pads when the `map` or its `pair` has the flag, this crate when the `map` or
   its `Array` does; no map in the game has any of the flags, so this is untested on real data.
4. **Reads stop at the object's end.** rabex reads from a stream over the whole file, so a value running past
   the object would read the next object's bytes.
5. **Tuples and maps left partly unread** (a tuple asking for fewer elements than the node has children, a map
   visitor that stops early) skip the rest, where rabex leaves the stream where it was.
6. **Sizes.** rabex caps LZ4 blocks at 64 MiB but not by the ratio, and relies on `Vec` growth for counts; this
   crate checks every size as in 1.4 and 2.5.
7. **Class names.** rabex knows about 900 ids, this crate 236. Both agree on every id of the game.
8. **No LZMA, no block-info padding, no other format versions** (1.1, 1.5, 2.1).
9. **The Unity version** is kept as the string in the file; rabex parses it, and fails to open a file whose
   version it cannot parse.
10. **The reference type hash** of a format 17 node (the field does not exist) is 0 here and absent in rabex.

The first oracle runs found, and these fixed: the 29 bundles of format 17 (only format 21 had been implemented),
three class names, and a panic from a hostile entry offset (`offset + size` overflowing) found by the hostile
input tests.

## How sensitive the oracle is

A comparison that finds nothing proves little unless it can find something. Three deliberate faults put into
the crate, one at a time, and the oracle run on the 16 demon bundles `dvl0x00` to `dvl0x0f`:

- no padding after strings: 8,286 object digests differ;
- `Texture2D` misspelled at its offset in the common strings table: every type tree with a `Texture2D` differs;
- no padding after `TypelessData`: no difference, also on `f015`, `f040`, `f060` and `f615`, which hold the
  odd-length textures. That is the finding in item 1 above: what follows them is zeros, so this fault cannot
  be seen through values, and the unit tests are what pins the padding down.
