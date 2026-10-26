# The subset

This crate exists to replace the git dependency on our fork of
[rabex](https://github.com/jakobhellermann/RustyAssetBundleEXtractor) (revision `56dd8a4`, MIT OR Apache-2.0) in
[nocturne-unpack](https://github.com/patodois/nocturne-unpack). The inventory below is what nocturne-unpack
used from rabex (`src/bundle.rs` and `src/install.rs`, the only files that named it), and what this crate has
for each. Nothing else of rabex was written: no writers, no builders, no TPK type tree database, no LZMA, no
UnityCN encryption, no format versions but the two the game ships.

| rabex, as nocturne-unpack called it | here |
|---|---|
| `BundleFileReader::from_reader(Cursor<Mmap>, ExtractionConfig::default().assume_recent_unity())` | `Bundle::new(Mmap)`; the Unity version is read from the file, so there is no config |
| `reader.files()` (`path`, `size`) | `bundle.entries()` |
| `reader.main_serializedfile()` | `bundle.main_serialized_entry()` |
| `reader.read_at_entry(&entry)` | `bundle.read_entry(&entry)` |
| `reader.read_at_entry_range(&entry, start, len)` (the fork's addition, for `.resS`) | `bundle.read_entry_range(&entry, start, len)` |
| `.files().iter().find(|f| f.path == name)` | `bundle.entry(name)` |
| `SerializedFile::from_reader(&mut Cursor<&[u8]>)` | `SerializedFile::parse(&[u8])` |
| `file.m_UnityVersion` (printed with `to_string`) | `file.unity_version` (the string as stored) |
| `file.m_EnableTypeTree` | `file.has_type_tree` |
| `file.externals_paths()` | `file.externals[i].path` |
| `file.objects()` (`m_PathID`, `m_ClassID.0`) | `file.objects()` (`path_id`, `class_id.0`) |
| `file.get_object_info(path_id)` (`m_Offset`, `m_Size`, `m_ClassID`) | `file.object(path_id)` (`offset`, `size`, `class_id`), `file.raw(data, info)` |
| `file.read::<T>(info, NullTypeTreeProvider, &mut cursor)` (serde, from the type tree stored in the file) | `file.read::<T>(data, info)` (the same, bounded to the object's bytes) |
| `file.get_typetree_for(info, &NullTypeTreeProvider)` and `TypeTreeNode::dump()` | `file.type_tree(info)` and `TypeTreeNode::dump()` |
| `ClassId(n).name()` | `ClassId(n).name()` |

The Rust types nocturne-unpack reads (all by field name, the rest of each object skipped) are
`GameObject`, `Transform`, `MeshFilter`, `MeshRenderer` and `SkinnedMeshRenderer` (one struct for both),
`Mesh` (with its `m_StreamData`), `Texture2D` (with `image data` and `m_StreamData`), `Material` (with `map`s read as
lists of pairs), `Shader`, `TextAsset`, `AnimationClip`, `Animator`, `Avatar` (a `map` of hash to string),
`AnimatorController` and `AssetBundle` (a `map` of path to `AssetInfo`), plus `serde_json::Value` for its `dump`
command, which goes through `deserialize_any`. The oracle compares all of them, and `deserialize_any` for
every object, on every file of the game.

For the Unity version the game uses: its bundles are written by Unity 2019.4.9f1 and 2019.4.10f1 (serialized
format 21), and 29 of them by Unity 2018.4.13f1 (format 17), which the first plan (2019.4 only) did not cover; the
oracle found them on its first run.
