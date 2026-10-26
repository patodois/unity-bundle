//! Serialized files: the objects of a bundle (or of a `level`, `.assets` file of a player), with
//! the type table that describes how each is laid out.
//!
//! Only format versions 17 (Unity 2018.4) and 21 (Unity 2019.4) and little-endian data are read:
//! those are the versions of the files the game ships. The header is big-endian whatever the data
//! is. See `docs/FORMATS.md`.

use std::collections::HashMap;

use serde::de::DeserializeOwned;

use crate::bundle::checked_count;
use crate::class_id::{ClassId, MONO_BEHAVIOUR};
use crate::cursor::Cursor;
use crate::de;
use crate::error::{invalid, Error, Result};
use crate::typetree::{self, TypeTreeNode};

/// The serialized file format version of Unity 2019.4.
pub const FORMAT_VERSION: u32 = 21;

/// The serialized file format version of Unity 2018.4.
pub const FORMAT_VERSION_2018_4: u32 = 17;

/// An entry of the type table: one class (or, for scripts, one script) with the type tree its
/// objects are laid out by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializedType {
    /// The class id of the objects of this type.
    pub class_id: ClassId,
    /// Whether the type tree was stripped from the build.
    pub is_stripped: bool,
    /// Index of the script for `MonoBehaviour` types, `-1` otherwise.
    pub script_type_index: i16,
    /// Hash of the script (present for `MonoBehaviour` types).
    pub script_id: [u8; 16],
    /// Hash of the type tree.
    pub old_type_hash: [u8; 16],
    /// The type tree; present when the file was written with type trees.
    pub type_tree: Option<TypeTreeNode>,
    /// Indices of the types this one depends on (for managed types).
    pub type_dependencies: Vec<i32>,
    /// For the types of managed references: class, namespace and assembly names.
    pub reference_names: Option<(String, String, String)>,
}

/// Where an object is and how it is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    /// The id of the object inside the file.
    pub path_id: i64,
    /// Where its data starts in the file (the data offset of the header already added).
    pub offset: u64,
    /// Size of its data.
    pub size: u32,
    /// Index into the type table.
    pub type_index: i32,
    /// The class id, from the type table.
    pub class_id: ClassId,
}

/// A file this one points into (`m_FileID` 1 is the first, 0 is the file itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct External {
    /// An always-empty string that precedes the GUID.
    pub temp_empty: String,
    /// GUID of the file.
    pub guid: [u8; 16],
    /// Kind of file (0 for an asset in the same project, 1 to 3 for others).
    pub kind: i32,
    /// Path, e.g. `archive:/CAB-0123.../CAB-0123...` or `Resources/unity_builtin_extra`.
    pub path: String,
}

/// The place in the file of a script (`MonoScript`) that a managed type refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptType {
    /// File index like a `PPtr`'s `m_FileID`.
    pub file_index: i32,
    /// Path id of the script object.
    pub path_id: i64,
}

/// A parsed serialized file: its tables. The objects' data stays in the buffer the file was
/// parsed from; [`SerializedFile::read`] takes that buffer again.
#[derive(Clone, Debug)]
pub struct SerializedFile {
    /// The format version of the file (`17` or `21`).
    pub format_version: u32,
    /// Offset where the object data starts.
    pub data_offset: u64,
    /// Size of the file as the header states it.
    pub file_size: u64,
    /// Size of the header plus metadata as the header states it.
    pub metadata_size: u32,
    /// The Unity version that wrote the file, e.g. `2019.4.9f1`.
    pub unity_version: String,
    /// The build target (`19` is Windows standalone 64-bit).
    pub target_platform: i32,
    /// Whether the type table carries type trees.
    pub has_type_tree: bool,
    /// The type table.
    pub types: Vec<SerializedType>,
    /// The objects, in file order.
    pub objects: Vec<ObjectInfo>,
    /// The scripts that managed types refer to.
    pub script_types: Vec<ScriptType>,
    /// The files the objects point into.
    pub externals: Vec<External>,
    /// Types of managed references.
    pub ref_types: Vec<SerializedType>,
    /// A free-form string at the end of the metadata (empty in the game's files).
    pub user_information: String,
    lookup: HashMap<i64, usize>,
}

impl SerializedFile {
    /// Parses the tables of a serialized file. `data` is the whole file, from its first byte.
    pub fn parse(data: &[u8]) -> Result<SerializedFile> {
        let mut cur = Cursor::new(data);
        let metadata_size = cur.u32_be("serialized file header")?;
        let file_size = cur.u32_be("serialized file header")?;
        let version = cur.u32_be("serialized file header")?;
        let data_offset = cur.u32_be("serialized file header")?;
        if version != FORMAT_VERSION && version != FORMAT_VERSION_2018_4 {
            return Err(Error::Unsupported(format!(
                "serialized file format version {version}, only {FORMAT_VERSION_2018_4} (Unity 2018.4) and {FORMAT_VERSION} (Unity 2019.4) are read"
            )));
        }
        let endianness = cur.u8("serialized file header")?;
        let _reserved = cur.array::<3>("serialized file header")?;
        if endianness != 0 {
            return Err(Error::Unsupported("big-endian serialized file".to_string()));
        }
        let data_offset = data_offset as u64;
        if data_offset > data.len() as u64 {
            return Err(invalid("data offset lies outside the file"));
        }

        let unity_version = cur.cstr("unity version")?;
        let target_platform = cur.i32_le("target platform")?;
        let has_type_tree = cur.bool_("type tree flag")?;

        let type_count = cur.i32_le("type count")?;
        // The smallest type record: class id, stripped flag, script index and the type hash
        let type_count = checked_count(type_count, 4 + 1 + 2 + 16, cur.remaining(), "type count")?;
        let mut types = Vec::with_capacity(type_count);
        for _ in 0..type_count {
            types.push(read_type(&mut cur, version, has_type_tree, false)?);
        }

        let object_count = cur.i32_le("object count")?;
        // path id 8, offset 4, size 4, type index 4
        let object_count = checked_count(object_count, 20, cur.remaining(), "object count")?;
        let mut objects = Vec::with_capacity(object_count);
        let mut lookup = HashMap::with_capacity(object_count);
        for i in 0..object_count {
            cur.align(4);
            let path_id = cur.i64_le("object")?;
            let offset = cur.u32_le("object")? as u64;
            let size = cur.u32_le("object")?;
            let type_index = cur.i32_le("object")?;
            let class_id = usize::try_from(type_index)
                .ok()
                .and_then(|t| types.get(t))
                .map(|t| t.class_id)
                .ok_or_else(|| invalid("object refers to a type that is not in the table"))?;
            let offset = offset
                .checked_add(data_offset)
                .ok_or_else(|| invalid("object offset overflows"))?;
            objects.push(ObjectInfo {
                path_id,
                offset,
                size,
                type_index,
                class_id,
            });
            // With a repeated path id (Unity never writes one) the last object wins
            lookup.insert(path_id, i);
        }

        let script_count = cur.i32_le("script type count")?;
        let script_count = checked_count(script_count, 12, cur.remaining(), "script type count")?;
        let mut script_types = Vec::with_capacity(script_count);
        for _ in 0..script_count {
            let file_index = cur.i32_le("script type")?;
            cur.align(4);
            let path_id = cur.i64_le("script type")?;
            script_types.push(ScriptType {
                file_index,
                path_id,
            });
        }

        let external_count = cur.i32_le("external count")?;
        // A NUL, 16 bytes of GUID, 4 of kind and a NUL, twice the strings at the least
        let external_count = checked_count(
            external_count,
            1 + 16 + 4 + 1,
            cur.remaining(),
            "external count",
        )?;
        let mut externals = Vec::with_capacity(external_count);
        for _ in 0..external_count {
            externals.push(External {
                temp_empty: cur.cstr("external")?,
                guid: cur.array::<16>("external")?,
                kind: cur.i32_le("external")?,
                path: cur.cstr("external")?,
            });
        }

        // Managed reference types exist from format version 20
        let mut ref_types = Vec::new();
        if version >= 20 {
            let ref_count = cur.i32_le("reference type count")?;
            let ref_count = checked_count(
                ref_count,
                4 + 1 + 2 + 16,
                cur.remaining(),
                "reference type count",
            )?;
            ref_types.reserve_exact(ref_count);
            for _ in 0..ref_count {
                ref_types.push(read_type(&mut cur, version, has_type_tree, true)?);
            }
        }

        let user_information = cur.cstr("user information")?;

        Ok(SerializedFile {
            format_version: version,
            data_offset,
            file_size: file_size as u64,
            metadata_size,
            unity_version,
            target_platform,
            has_type_tree,
            types,
            objects,
            script_types,
            externals,
            ref_types,
            user_information,
            lookup,
        })
    }

    /// The object with this path id.
    pub fn object(&self, path_id: i64) -> Option<&ObjectInfo> {
        self.lookup.get(&path_id).map(|&i| &self.objects[i])
    }

    /// The type tree an object is laid out by.
    pub fn type_tree(&self, info: &ObjectInfo) -> Option<&TypeTreeNode> {
        usize::try_from(info.type_index)
            .ok()
            .and_then(|t| self.types.get(t))
            .and_then(|t| t.type_tree.as_ref())
    }

    /// The stored bytes of an object. `data` is the buffer the file was parsed from.
    pub fn raw<'a>(&self, data: &'a [u8], info: &ObjectInfo) -> Result<&'a [u8]> {
        let start = usize::try_from(info.offset).map_err(|_| invalid("object offset"))?;
        let end = start
            .checked_add(info.size as usize)
            .filter(|&e| e <= data.len())
            .ok_or_else(|| invalid("object lies outside the file"))?;
        Ok(&data[start..end])
    }

    /// Deserializes an object into any `serde` type, through the type tree stored for its type.
    /// Fields are matched by the names in the type tree.
    pub fn read<T: DeserializeOwned>(&self, data: &[u8], info: &ObjectInfo) -> Result<T> {
        let tree = self.type_tree(info).ok_or(Error::NoTypeTree)?;
        let bytes = self.raw(data, info)?;
        de::from_slice(bytes, tree)
    }

    /// Like [`SerializedFile::read`], for an object found by its path id.
    pub fn read_path_id<T: DeserializeOwned>(&self, data: &[u8], path_id: i64) -> Result<T> {
        let info = self.object(path_id).ok_or(Error::NoObject(path_id))?;
        self.read(data, info)
    }
}

fn read_type(
    cur: &mut Cursor<'_>,
    version: u32,
    has_type_tree: bool,
    is_ref_type: bool,
) -> Result<SerializedType> {
    let class_id = ClassId(cur.i32_le("type")?);
    let is_stripped = cur.bool_("type")?;
    let script_type_index = cur.i16_le("type")?;
    let mut script_id = [0u8; 16];
    if (is_ref_type && script_type_index >= 0) || class_id.0 == MONO_BEHAVIOUR {
        script_id = cur.array::<16>("type script hash")?;
    }
    let old_type_hash = cur.array::<16>("type hash")?;
    let mut type_tree = None;
    let mut type_dependencies = Vec::new();
    let mut reference_names = None;
    if has_type_tree {
        type_tree = Some(typetree::read_blob(cur, version)?);
        if version < 21 {
            // Type dependencies and managed type names are written from version 21
        } else if is_ref_type {
            reference_names = Some((
                cur.cstr("reference class name")?,
                cur.cstr("reference namespace")?,
                cur.cstr("reference assembly")?,
            ));
        } else {
            let n = cur.i32_le("type dependency count")?;
            let n = checked_count(n, 4, cur.remaining(), "type dependency count")?;
            type_dependencies.reserve_exact(n);
            for _ in 0..n {
                type_dependencies.push(cur.i32_le("type dependency")?);
            }
        }
    }
    Ok(SerializedType {
        class_id,
        is_stripped,
        script_type_index,
        script_id,
        old_type_hash,
        type_tree,
        type_dependencies,
        reference_names,
    })
}
