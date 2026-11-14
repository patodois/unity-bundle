//! Type trees: the description of an object's fields that a serialized file carries, and the
//! reader of the blob they are stored in.
//!
//! Credit: the classification of nodes into kinds (`Kind`, `Kind::classify`) follows the structure of
//! rabex's `TypeTreeNode::classify` and `TypetreeNodeKind` (Rudolf Kolbe and Jakob Hellermann,
//! MIT OR Apache-2.0), which the author had read; no text or expressions were pasted. See the README.

use crate::common_strings;
use crate::cursor::Cursor;
use crate::error::{invalid, reserve, Error, Result};

/// The bit of the meta flags that makes the reader skip to the next multiple of 4 after the node.
pub const ALIGN_BYTES_FLAG: i32 = 1 << 14;

/// What a node stands for when data is read through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Float,
    Double,
    Char,
    String,
    Map,
    /// `TypelessData`: a length and that many bytes
    Typeless,
    /// A node with no children that is not a primitive
    Empty,
    /// A `vector`, `set`, ...: one child, the `Array`
    ArrayWrapper,
    /// The `Array` itself
    Array,
    Struct,
    /// `ReferencedObject` and friends: not read
    Referenced,
    /// `Type`: nothing is stored
    TypeMarker,
}

impl Kind {
    fn classify(type_name: &str, children: &[TypeTreeNode]) -> Kind {
        match type_name {
            "bool" => Kind::Bool,
            "UInt8" => Kind::U8,
            "UInt16" | "unsigned short" => Kind::U16,
            "UInt32" | "unsigned int" | "Type*" => Kind::U32,
            "UInt64" | "unsigned long long" | "FileSize" => Kind::U64,
            "SInt8" => Kind::I8,
            "SInt16" | "short" => Kind::I16,
            "SInt32" | "int" => Kind::I32,
            "SInt64" | "long long" => Kind::I64,
            "float" => Kind::Float,
            "double" => Kind::Double,
            "char" => Kind::Char,
            "string" => Kind::String,
            "map" => Kind::Map,
            "Type" => Kind::TypeMarker,
            "Array" => Kind::Array,
            "TypelessData" => Kind::Typeless,
            "ReferencedObject" | "ReferencedObjectData" | "ManagedReferencesRegistry" => {
                Kind::Referenced
            }
            _ => match children.len() {
                0 => Kind::Empty,
                1 if children[0].type_name == "Array" => Kind::ArrayWrapper,
                _ => Kind::Struct,
            },
        }
    }

    /// The least number of bytes a value of this kind takes (without padding).
    fn min_size(self, children: &[TypeTreeNode]) -> usize {
        match self {
            Kind::Bool | Kind::U8 | Kind::I8 | Kind::Char => 1,
            Kind::U16 | Kind::I16 => 2,
            Kind::U32 | Kind::I32 | Kind::Float => 4,
            Kind::U64 | Kind::I64 | Kind::Double => 8,
            // a length
            Kind::String | Kind::Map | Kind::Typeless | Kind::ArrayWrapper | Kind::Array => 4,
            Kind::Struct => children
                .iter()
                .fold(0usize, |s, c| s.saturating_add(c.min_size)),
            Kind::Empty | Kind::Referenced | Kind::TypeMarker => 0,
        }
    }
}

/// One node of a type tree: a field, with its type, and the fields inside it.
///
/// Reading an object walks the tree: a node says what to read and in what order its children
/// come. The fields are read through accessors, so that a node's children cannot be changed
/// behind the back of what was worked out from them when it was built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeTreeNode {
    pub(crate) type_name: String,
    pub(crate) name: String,
    pub(crate) version: u16,
    pub(crate) type_flags: u8,
    pub(crate) byte_size: i32,
    pub(crate) index: i32,
    pub(crate) meta_flags: i32,
    pub(crate) ref_type_hash: u64,
    pub(crate) children: Vec<TypeTreeNode>,
    pub(crate) kind: Kind,
    pub(crate) min_size: usize,
}

impl TypeTreeNode {
    /// Builds a node and works out how it is read.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        type_name: String,
        name: String,
        version: u16,
        type_flags: u8,
        byte_size: i32,
        index: i32,
        meta_flags: i32,
        ref_type_hash: u64,
        children: Vec<TypeTreeNode>,
    ) -> TypeTreeNode {
        let kind = Kind::classify(&type_name, &children);
        let min_size = kind.min_size(&children);
        TypeTreeNode {
            type_name,
            name,
            version,
            type_flags,
            byte_size,
            index,
            meta_flags,
            ref_type_hash,
            children,
            kind,
            min_size,
        }
    }

    /// The type, e.g. `float`, `string`, `PPtr<Material>`, `vector`.
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// The name of the field, e.g. `m_Name`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Version of the type (`m_Version`).
    pub fn version(&self) -> u16 {
        self.version
    }

    /// `m_TypeFlags`; bit 0 is set on arrays.
    pub fn type_flags(&self) -> u8 {
        self.type_flags
    }

    /// Size in bytes of a fixed-size value, `-1` for a variable one.
    pub fn byte_size(&self) -> i32 {
        self.byte_size
    }

    /// Position of the node in the pre-order walk of the tree.
    pub fn index(&self) -> i32 {
        self.index
    }

    /// `m_MetaFlag`; see [`ALIGN_BYTES_FLAG`].
    pub fn meta_flags(&self) -> i32 {
        self.meta_flags
    }

    /// Hash of the referenced type, for managed references (zero otherwise).
    pub fn ref_type_hash(&self) -> u64 {
        self.ref_type_hash
    }

    /// The nodes inside this one, in the order they are stored.
    pub fn children(&self) -> &[TypeTreeNode] {
        &self.children
    }

    /// Whether the data is padded to a multiple of 4 after this node.
    pub fn aligns(&self) -> bool {
        self.meta_flags & ALIGN_BYTES_FLAG != 0
    }

    /// The nodes as an indented list of `type name`, two spaces per level.
    pub fn dump(&self) -> String {
        fn go(n: &TypeTreeNode, out: &mut String, depth: usize) {
            for _ in 0..depth {
                out.push_str("  ");
            }
            out.push_str(&n.type_name);
            out.push(' ');
            out.push_str(&n.name);
            out.push('\n');
            for c in &n.children {
                go(c, out, depth + 1);
            }
        }
        let mut s = String::new();
        go(self, &mut s, 0);
        s
    }
}

/// Reads one type tree from the blob format used by serialized file versions 12 and later (the
/// game's files are versions 17 and 21): a count, the size of a string buffer, the nodes as
/// fixed-size records in pre-order with their depth, and the string buffer. A record is 24 bytes,
/// and 32 from version 19 on, which adds the hash of the referenced type.
pub(crate) fn read_blob(cur: &mut Cursor<'_>, format_version: u32) -> Result<TypeTreeNode> {
    let node_record: usize = if format_version >= 19 { 32 } else { 24 };
    let node_count = cur.i32_le("type tree node count")?;
    let string_size = cur.i32_le("type tree string size")?;
    let node_count = usize::try_from(node_count).map_err(|_| invalid("negative node count"))?;
    let string_size = usize::try_from(string_size).map_err(|_| invalid("negative string size"))?;
    if node_count == 0 {
        return Err(invalid("a type tree with no nodes"));
    }
    // Check both sizes against what remains before anything is allocated
    let node_bytes = node_count.checked_mul(node_record).ok_or(Error::TooLarge {
        what: "type tree node count",
        value: node_count as u64,
        limit: (cur.remaining() / node_record) as u64,
    })?;
    if node_bytes > cur.remaining() {
        return Err(Error::TooLarge {
            what: "type tree node count",
            value: node_count as u64,
            limit: (cur.remaining() / node_record) as u64,
        });
    }
    let nodes = cur.bytes(node_bytes, "type tree nodes")?;
    let strings = cur.bytes(string_size, "type tree strings")?;

    struct Raw {
        level: u8,
        type_name: String,
        name: String,
        version: u16,
        type_flags: u8,
        byte_size: i32,
        index: i32,
        meta_flags: i32,
        ref_type_hash: u64,
    }
    // Many nodes can name the same string, and every node gets its own copy, so the strings
    // copied for one tree are limited to what a real tree needs with room to spare: 64 bytes for
    // each node and the size of the tree's own string buffer.
    let string_budget = node_count.saturating_mul(64).saturating_add(string_size);
    let mut string_used = 0usize;
    let mut take_string = |offset: u32| -> Result<String> {
        let s = locate_string(strings, offset)?;
        string_used = string_used.saturating_add(s.len());
        if string_used > string_budget {
            return Err(Error::TooLarge {
                what: "type tree string bytes",
                value: string_used as u64,
                limit: string_budget as u64,
            });
        }
        // A NUL-terminated string, as in `Cursor::cstr`: its size is not a count the file states,
        // and the budget above bounds what all the copies of a tree come to
        Ok(s.to_string())
    };
    let mut raws: Vec<Raw> = Vec::new();
    reserve(&mut raws, node_count, "type tree nodes")?;
    let mut rec = Cursor::new(nodes);
    for _ in 0..node_count {
        let version = rec.u16_le("type tree node")?;
        let level = rec.u8("type tree node")?;
        let type_flags = rec.u8("type tree node")?;
        let type_off = rec.u32_le("type tree node")?;
        let name_off = rec.u32_le("type tree node")?;
        let byte_size = rec.i32_le("type tree node")?;
        let index = rec.i32_le("type tree node")?;
        let meta_flags = rec.i32_le("type tree node")?;
        let ref_type_hash = if format_version >= 19 {
            rec.u64_le("type tree node")?
        } else {
            0
        };
        raws.push(Raw {
            level,
            type_name: take_string(type_off)?,
            name: take_string(name_off)?,
            version,
            type_flags,
            byte_size,
            index,
            meta_flags,
            ref_type_hash,
        });
    }

    // The levels give the nesting: a node is the child of the nearest earlier node that is one
    // level up, and the first node is the only one at the root level.
    let root_level = raws[0].level;
    let mut parent: Vec<usize> = Vec::new();
    reserve(&mut parent, node_count, "type tree nodes")?;
    parent.resize(node_count, usize::MAX);
    // How many children each node has, so that each list of children is allocated once and at
    // its size (a list that grew by pushing would hold room for four nodes to keep one)
    let mut child_count: Vec<usize> = Vec::new();
    reserve(&mut child_count, node_count, "type tree nodes")?;
    child_count.resize(node_count, 0);
    let mut stack: Vec<usize> = vec![0];
    for i in 1..node_count {
        let level = raws[i].level;
        while let Some(&top) = stack.last() {
            if raws[top].level >= level {
                stack.pop();
            } else {
                break;
            }
        }
        let Some(&p) = stack.last() else {
            return Err(invalid("type tree has a second root"));
        };
        if raws[p].level.checked_add(1) != Some(level) || level <= root_level {
            return Err(invalid("type tree levels skip a level"));
        }
        parent[i] = p;
        child_count[p] += 1;
        stack.push(i);
    }

    // Build from the last node backwards: when a node's turn comes, all of its children (which
    // have larger indices) are complete, collected in reverse order
    let mut pending: Vec<Vec<TypeTreeNode>> = Vec::new();
    reserve(&mut pending, node_count, "type tree nodes")?;
    pending.resize_with(node_count, Vec::new);
    let mut root = None;
    for i in (0..node_count).rev() {
        let mut children = std::mem::take(&mut pending[i]);
        children.reverse();
        let raw = std::mem::replace(
            &mut raws[i],
            Raw {
                level: 0,
                type_name: String::new(),
                name: String::new(),
                version: 0,
                type_flags: 0,
                byte_size: 0,
                index: 0,
                meta_flags: 0,
                ref_type_hash: 0,
            },
        );
        let node = TypeTreeNode::new(
            raw.type_name,
            raw.name,
            raw.version,
            raw.type_flags,
            raw.byte_size,
            raw.index,
            raw.meta_flags,
            raw.ref_type_hash,
            children,
        );
        if i == 0 {
            root = Some(node);
        } else {
            let siblings = &mut pending[parent[i]];
            if siblings.capacity() == 0 {
                reserve(siblings, child_count[parent[i]], "type tree nodes")?;
            }
            siblings.push(node);
        }
    }
    root.ok_or_else(|| invalid("a type tree with no nodes"))
}

/// The string a node names, in its tree's buffer or in the common strings table.
fn locate_string(strings: &[u8], offset: u32) -> Result<&str> {
    let common = offset & 0x8000_0000 != 0;
    let (buf, off) = if common {
        (
            common_strings::BUFFER.as_bytes(),
            (offset & 0x7fff_ffff) as usize,
        )
    } else {
        (strings, offset as usize)
    };
    // A common string is one of the table's entries, so its offset must be where one starts
    if common && !(off == 0 || (off < buf.len() && buf[off - 1] == 0)) {
        return Err(invalid(format!("unknown common string at offset {off}")));
    }
    let rest = buf.get(off..).ok_or_else(|| {
        invalid(format!(
            "type tree string offset {offset:#x} is out of range"
        ))
    })?;
    let len = rest.iter().position(|&b| b == 0).ok_or_else(|| {
        invalid(format!(
            "type tree string at {offset:#x} has no terminating NUL"
        ))
    })?;
    std::str::from_utf8(&rest[..len]).map_err(|_| invalid("type tree string is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_node_blob(type_offset: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&1i32.to_le_bytes()); // one node
        b.extend_from_slice(&0i32.to_le_bytes()); // no local strings
        b.extend_from_slice(&1u16.to_le_bytes()); // version
        b.push(0); // level
        b.push(0); // type flags
        b.extend_from_slice(&type_offset.to_le_bytes());
        b.extend_from_slice(&(0x8000_0000u32 | 427).to_le_bytes()); // m_Name
        b.extend_from_slice(&(-1i32).to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b
    }

    #[test]
    fn common_strings_are_looked_up_by_offset() {
        let blob = one_node_blob(0x8000_0000 | 874);
        let node = read_blob(&mut Cursor::new(&blob), 21).unwrap();
        assert_eq!(
            (node.type_name.as_str(), node.name.as_str()),
            ("Texture2D", "m_Name")
        );
        let first = one_node_blob(0x8000_0000);
        assert_eq!(
            read_blob(&mut Cursor::new(&first), 21).unwrap().type_name,
            "AABB"
        );
    }

    #[test]
    fn an_unknown_common_string_is_an_error_naming_the_offset() {
        // inside an entry, past the end of the table, and far past it
        for off in [1u32, 876, 1057, 1161, 0x7fff_ffff] {
            let blob = one_node_blob(0x8000_0000 | off);
            let e = read_blob(&mut Cursor::new(&blob), 21).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!("invalid data: unknown common string at offset {off}")
            );
        }
    }
}
