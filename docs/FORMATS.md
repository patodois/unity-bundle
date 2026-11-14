# Formats

What `unity-bundle` reads, written as facts. Each fact says where it comes from:

- **game**: seen in the files of Shin Megami Tensei III Nocturne HD Remaster (Steam, Windows build): 2,251 bundles
  in `smt3hd_Data/StreamingAssets/PC`, 837 in `dlc/*` (3,088 files, 12.1 GB), and 9 loose serialized files
  in `smt3hd_Data`. The numbers come from the oracle run in `tools/oracle/REPORT.txt` (this crate against
  rabex, on every file) and from `tools/survey.py` (`tools/SURVEY.txt`), an independent script in Python with its own
  LZ4 decoder, sharing no code with the crate, that read the header, the block info and the first block of every
  bundle by seeking.
- **spec**: a public description, cited, or a public table that was compared entry by entry (with its revision).
- **choice**: a decision of this crate, with its reason.

All numbers in the bundle header and block info are **big-endian**; in a serialized file the first 20
bytes are big-endian and everything after them is **little-endian**.

The examples are from `dvl0x01`, a 2.7 MB demon bundle written by Unity 2019.4.9f1. `cargo run --example
inspect -- FILE --hex 64 --tree 28` prints the first 64 bytes of the file as it is on disk (the hexdump in 1.1),
the fields of the header, the files, the counts of the serialized file and the type tree of class 28. It does not
dump decompressed data: the hexdumps of the block info (1.2) and of the serialized file (2.2) were made with a
separate script.

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

**choice**: The rabex fork this crate was compared with accepts zero padding to a multiple of 16 before the
block info in version 6 bundles of 2019.4 (its comment says AssetStudio does the same; AssetStudio at the
revision named in section 3 aligns only for format version 7 and later, so that was not confirmed). No file of
the game has any, so it is not read: such a bundle fails with a decompression error rather than being guessed
at. When the block info is LZ4 and every byte from the header's last field to the next multiple of 16 is zero, the
error adds that this looks like a padded block info (an LZ4 stream of a block info cannot start with a zero byte).
A block info stored without compression (flags `0x40`; the game has none) cannot be checked this way, because its
first 16 bytes are zero in any case: read from where the header ends, a padded one can pass the zeros of its
padding and of its hash for the hash and the two counts, and the bundle then opens with no blocks and no files
(it does with 15 bytes of padding).

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

*String budget.* Every node gets its own copy of its two strings, and many nodes can name the same offset of the
string buffer, so the strings copied for one tree are limited to 64 bytes per node plus the size of the tree's
string buffer (`Error::TooLarge`, "type tree string bytes"). Without the limit a 264 KB file of 2,000 nodes
and a 200,000-byte buffer, every offset 0, asks for 763 MiB; the test `tests/hostile.rs` has that file. The
game's trees are far below it.

*Common strings.* The game's trees use 64 of the 101 entries of the table (the instrumented run is in
`REPORT.txt`): `AABB`, `AnimationClip`, `AnimationCurve`, `Array`, `Base`, `BitField`, `bool`, `char`,
`ColorRGBA`, `data`, `double`, `first`, `float`, `GameObject`, `GUID`, `int`, `map`, `Matrix4x4f`,
`MonoBehaviour`, `MonoScript`, `m_Curve`, `m_Enabled`, `m_GameObject`, `m_Index`, `m_Name`, `m_Script`, `m_Type`,
`m_Version`, `pair`, `PPtr<Component>`, `PPtr<GameObject>`, `PPtr<Material>`, `PPtr<MonoBehaviour>`,
`PPtr<MonoScript>`, `PPtr<Object>`, `PPtr<Sprite>`, `PPtr<Texture>`, `PPtr<Texture2D>`, `PPtr<Transform>`,
`Quaternionf`, `Rectf`, `RectOffset`, `second`, `set`, `size`, `SInt16`, `SInt64`, `SInt8`, `staticvector`,
`string`, `TextAsset`, `Texture2D`, `Transform`, `TypelessData`, `UInt16`, `UInt64`, `UInt8`, `unsigned int`,
`vector`, `Vector2f`, `Vector3f`, `Vector4f`, `Gradient`, `Type*`. Every one decodes to the same string in this
crate and in rabex.

All 101 entries of `src/common_strings.rs` (text and offset) are identical in two separately maintained public
tables, compared by `tools/check_tables.py` (output in `tools/TABLES.txt`): rabex's `src/commonstring.rs`
(revision `56dd8a4d8d5048b6ee7664eadc89d98343c66002`, 113 entries, generated there from the AssetRipper TPK
database) and AssetStudio's `AssetStudio/CommonString.cs` (revision `d158e864b556b5970709c2a52e47944d53aa98a2`,
109 entries). Of the 101, the game uses 64 (above); the other 37 (for example `SInt32`, `UInt32`, `m_MetaFlag`,
`m_IsArray`) are confirmed by those two sources only, and the offsets of the later entries, which the game does
use, confirm that no earlier entry has a wrong length. UnityPy's table was not compared: it is read from a
binary TPK file (`UnityPy/resources/lzma.tpk`) that is not readable as text in its repository. The table stops
at `Type*` (offset 1051), though both sources go on (`Hash128` in AssetStudio, `LoadableReference` in rabex):
the game does not use anything past `Type*`, and stores the later names (`int2_storage`, `m_PrefabAsset`, ...)
in each tree's own string buffer. An offset past `Type*`, or inside an entry, is an error, `unknown common
string at offset N`.

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

The number Unity gives each kind of object. `src/class_id.rs` has 233 names. Each is in the class id table of
rabex (revision `56dd8a4d...`, `src/objects/class_id.rs`) under the same number, and 232 of them are also in
AssetStudio's `ClassIDType` enum (revision `d158e864...`, `AssetStudio/ClassIDType.cs`); `tools/check_tables.py`
does the comparison (`tools/TABLES.txt`). The one name AssetStudio does not confirm is 94: rabex has
`ShaderNameRegistry` and AssetStudio, which is older, `ScriptMapper`; the game has an object of that class
only in a loose file without type trees, so it cannot decide, and rabex's name (which follows the Unity
version) is used. Three entries of an earlier draft (109, 134, 197) were dropped because the two tables
disagree on them or only one has them. The names were first written from the author's memory of Unity's class
ID reference, which was not fetched or read in this work, and kept only after this comparison.

**game**: the objects of the bundles belong to 41 classes (ids 1, 4, 20, 21, 23, 28, 33, 43, 48, 49, 58, 61,
65, 74, 81, 83, 84, 89, 90, 91, 95, 108, 111, 114, 115, 120, 136, 137, 142, 198, 199, 200, 213, 222, 223, 224,
225, 290, 329 and the two large ids 1183024399, `LookAtConstraint`, and 1773428102, `ParentConstraint`). For
each class with a type tree, the root of the tree is named like the class (`Texture2D`, `SkinnedMeshRenderer`,
...), except for `MonoBehaviour` (114), whose roots carry the C# class name; and rabex's table gives the same name
for every id the game uses. Ids that appear only in the loose files, which have no type trees, are checked
against the two tables alone: 94, 937362698 (`VFXManager`) and 1403656975 (`StreamingManager`).

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
| a node whose only child is an `Array` (`vector`, `set`, `staticvector`, ...) | u32 count, then that many values of the type of the `Array`'s second child |
| an `Array` in any other place (beside other children, or as the root) | not read (an error); every object of the game is read without meeting one |
| `map` | an array whose elements are `pair`s: u32 count, then the `first` and the `second` value of each |
| `Type` | nothing |
| any other node with children | its children, in order |
| any other node | nothing |
| `ReferencedObject`, `ReferencedObjectData`, `ManagedReferencesRegistry` | not read (an error) |

After a node whose `meta flags` have bit 14, padding to a multiple of 4. A `string` is always padded, whatever
its flags say (in the game the flag is on all 98,212 of them, so the oracle cannot tell the two rules apart; Unity,
AssetStudio and rabex pad strings unconditionally). A `TypelessData` is padded when the node (or its first child)
has the bit, a `vector` when it or its `Array` child has it. For a `map` see the known inconsistency below.

**Known inconsistency** (padding of a `map` whose `pair` node has bit 14). The three ways a `map` is consumed do
not agree when the flag is on the `pair`:

- skipped (a field the Rust type does not name, or `IgnoredAny`): the `first` and `second` of every entry are
  read with their own flags, the `pair`'s flag is not applied after an entry, and after the last entry there is
  one padding if the `map`, its `Array` or its `pair` has the bit;
- read as a map (`HashMap`, `BTreeMap`, `serde_json::Value`, anything through `deserialize_map` or
  `deserialize_any`): the same as skipped, and so are the entries a map visitor leaves unread;
- read as a sequence (`Vec<(K, V)>`, or a sequence of structs with `first` and `second`): each element is read
  as the `pair` node, so there is a padding after every entry if the `pair` has the bit, and after the last entry
  one padding if the `map` or its `Array` has the bit (the `pair`'s flag is not looked at there).

With the flag on the `pair` only, a map of `UInt8` to `UInt8` with two entries takes 4 + 4 bytes skipped or read
as a map, and 4 + 8 read as a sequence. When the `pair` does not have the bit the three agree. No file of the
game exercises this: none of its 11,859 `map` nodes has the bit on the `map`, its `Array` or its `pair` (the table
in 2.5), so which of the two Unity writes is not known here, and the behaviour is left as it is until a file
shows it.

**choice** (counts bounded by the data): an array's, map's, string's or typeless data's count is refused when it
exceeds the bytes that remain in the object divided by the least size of an element (at least 1, even for
types that take no bytes). Elements that take no bytes still cost work (an array of arrays of empty structs
multiplies), so the number of values visited while reading an object is limited too, to 16 per byte of the object
plus 1,024; a real object visits about one value per byte at most (the oracle reads all 1.47 million objects of
the game within it), and a bigger demand is `Error::TooLarge` ("values to visit"). A tree 255 levels deep (the most a node record can state) is read without
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
2. **Padding after strings.** The same: both always pad. (An earlier version of this crate padded only when a
   flag said so; every string of the game has one, and the oracle gave the same result with both rules.)
3. **Padding after maps.** rabex pads when the `map` or its `pair` has the flag; this crate looks at the `map`,
   its `Array` and its `pair`, and not in the same way on every path (the known inconsistency in section 4). No
   map in the game has any of the flags, so this is untested on real data.
4. **Reads stop at the object's end.** rabex reads from a stream over the whole file, so a value running past
   the object would read the next object's bytes.
5. **Tuples and maps left partly unread** (a tuple asking for fewer elements than the node has children, a map
   visitor that stops early) skip the rest, where rabex leaves the stream where it was.
6. **Sizes.** rabex caps LZ4 blocks at 64 MiB but not by the ratio, and relies on `Vec` growth for counts; this
   crate checks every size as in 1.4 and 2.5.
7. **Class names.** rabex has 413 names, this crate 233 (those on which rabex and AssetStudio agree, plus 94).
   Both agree on every id of the game.
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
