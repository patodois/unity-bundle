//! A `serde` deserializer that reads object data through a type tree.
//!
//! Credit: the way tree nodes serve `serde` types, the shape of `deserialize_any` (one arm per kind of
//! node), and the helper deserializers for field names (`KeyDe`), map keys (`MapKeyDe`) and typeless
//! bytes (`ByteSeq`) follow the structure of rabex's `serde_typetree::de` (`FixedKeyDeserializer`,
//! `LenientMapKeyDeserializer`, `ByteSeqDeserializer`; Rudolf Kolbe and Jakob Hellermann, MIT OR
//! Apache-2.0), which the author had read; no text or expressions were pasted. See the README.
//!
//! The tree says what comes next, so a Rust type does not have to list every field: a struct
//! asks for the fields it names, matched by the field names of the tree, and the rest is
//! skipped. The mapping of tree nodes to `serde` types is documented in `docs/FORMATS.md`:
//!
//! * a primitive node (`bool`, `UInt8`, `SInt32`, `float`, ...) gives the `serde` primitive of the
//!   same type, and nothing else (a `float` node cannot be read as `u32`);
//! * a `string` node gives a `String`, or a byte buffer when the bytes are not UTF-8;
//! * a `vector`, `set`, ... node (and a `map`) gives a sequence; a `map` node is a sequence of
//!   two-element tuples `(key, value)`, or a `serde` map whose keys are strings or `UInt32`;
//! * a `TypelessData` node gives a sequence of `u8`;
//! * any other node is a struct, or a tuple of its children;
//! * an `Option` is always `Some`: a type tree has no optional fields (use `#[serde(default)]`
//!   for a field the tree may not have).

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor,
};

use crate::cursor::Cursor;
use crate::error::{copy_of, Error, Result};
use crate::typetree::{Kind, TypeTreeNode};

/// How many values may be visited for each byte of an object, plus a constant. A real object visits
/// about one value per byte at most; values that take no bytes (an array of empty structs) would
/// otherwise let a few bytes ask for any number of visits.
const VISITS_PER_BYTE: usize = 16;
const VISITS_BASE: usize = 1024;

/// Deserializes an object from its bytes, through its type tree.
///
/// Fields are matched by name, so a struct (or a tuple, in the tree's order) can name only the
/// ones it wants. Limits of the mapping: `#[serde(flatten)]` and a `HashMap` read from a struct
/// node fail (a struct node is not a `map` node), `#[serde(deny_unknown_fields)]` never fires
/// (fields the type does not name are skipped before it could see them), `Option` is always
/// `Some`, `()` reads nothing, and enums are refused.
pub fn from_slice<T: DeserializeOwned>(bytes: &[u8], tree: &TypeTreeNode) -> Result<T> {
    let budget = bytes
        .len()
        .saturating_mul(VISITS_PER_BYTE)
        .saturating_add(VISITS_BASE);
    let mut rd = Cursor::with_budget(bytes, budget);
    T::deserialize(De {
        node: tree,
        rd: &mut rd,
    })
}

struct De<'t, 'b, 'r> {
    node: &'t TypeTreeNode,
    rd: &'r mut Cursor<'b>,
}

fn invalid_type(node: &TypeTreeNode, expected: &str) -> Error {
    Error::Deserialize(format!(
        "invalid type: {} `{}`, expected {expected}",
        node.type_name, node.name
    ))
}

fn bad_tree(node: &TypeTreeNode, what: &str) -> Error {
    Error::Deserialize(format!(
        "malformed type tree at {} `{}`: {what}",
        node.type_name, node.name
    ))
}

const OBJECT: &str = "object data";

/// The `Array` child of a `vector`-like node and the node of its elements.
fn array_of(node: &TypeTreeNode) -> Result<(&TypeTreeNode, &TypeTreeNode)> {
    match node.children.as_slice() {
        [array] if array.type_name == "Array" => match array.children.as_slice() {
            [_, item] => Ok((array, item)),
            _ => Err(bad_tree(node, "an Array without a size and an element")),
        },
        _ => Err(invalid_type(node, "a list")),
    }
}

/// Reads an element count and checks that the elements can fit in what remains of the data.
fn read_count(rd: &mut Cursor<'_>, item_min_size: usize, what: &'static str) -> Result<usize> {
    let n = rd.u32_le(what)? as usize;
    // An element takes at least a byte, even when its type needs none, so that a count cannot
    // make the reader loop longer than the data is long
    if n.saturating_mul(item_min_size.max(1)) > rd.remaining() {
        return Err(Error::TooLarge {
            what,
            value: n as u64,
            limit: (rd.remaining() / item_min_size.max(1)) as u64,
        });
    }
    Ok(n)
}

/// Whether the padding after typeless data is present.
fn aligns_with_child(node: &TypeTreeNode) -> bool {
    node.aligns() || node.children.first().is_some_and(|c| c.aligns())
}

/// Advances over a value without producing it.
fn skip(node: &TypeTreeNode, rd: &mut Cursor<'_>) -> Result<()> {
    rd.spend(1, "values to visit")?;
    match node.kind {
        Kind::Bool | Kind::U8 | Kind::I8 | Kind::Char => skip_fixed(node, rd, 1),
        Kind::U16 | Kind::I16 => skip_fixed(node, rd, 2),
        Kind::U32 | Kind::I32 | Kind::Float => skip_fixed(node, rd, 4),
        Kind::U64 | Kind::I64 | Kind::Double => skip_fixed(node, rd, 8),
        Kind::String => {
            let n = rd.u32_le(OBJECT)? as usize;
            rd.bytes(n, OBJECT)?;
            // A string is always padded
            rd.align(4);
            Ok(())
        }
        Kind::Typeless => {
            let n = rd.u32_le(OBJECT)? as usize;
            rd.bytes(n, OBJECT)?;
            if aligns_with_child(node) {
                rd.align(4);
            }
            Ok(())
        }
        Kind::Map => {
            let (array, pair) = array_of(node)?;
            let [key, value] = match pair.children.as_slice() {
                [k, v] => [k, v],
                _ => return Err(bad_tree(node, "a map whose pairs do not have two fields")),
            };
            let n = read_count(rd, pair.min_size, "map size")?;
            for _ in 0..n {
                skip(key, rd)?;
                skip(value, rd)?;
            }
            if node.aligns() || array.aligns() || pair.aligns() {
                rd.align(4);
            }
            Ok(())
        }
        Kind::ArrayWrapper => {
            let (array, item) = array_of(node)?;
            let n = read_count(rd, item.min_size, "array length")?;
            for _ in 0..n {
                skip(item, rd)?;
            }
            if node.aligns() || array.aligns() {
                rd.align(4);
            }
            Ok(())
        }
        Kind::Array => Err(invalid_type(node, "a list")),
        Kind::Struct => {
            for c in &node.children {
                skip(c, rd)?;
            }
            if node.aligns() {
                rd.align(4);
            }
            Ok(())
        }
        Kind::Empty | Kind::TypeMarker => Ok(()),
        Kind::Referenced => Err(Error::Unsupported(format!(
            "managed references ({} `{}`)",
            node.type_name, node.name
        ))),
    }
}

fn skip_fixed(node: &TypeTreeNode, rd: &mut Cursor<'_>, n: usize) -> Result<()> {
    rd.bytes(n, OBJECT)?;
    if node.aligns() {
        rd.align(4);
    }
    Ok(())
}

macro_rules! primitive {
    ($method:ident, $kind:ident, $expected:literal, $read:ident, $visit:ident) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
            if self.node.kind != Kind::$kind {
                return Err(invalid_type(self.node, $expected));
            }
            let v = self.rd.$read(OBJECT)?;
            if self.node.aligns() {
                self.rd.align(4);
            }
            visitor.$visit(v)
        }
    };
}

impl<'de, 't, 'b, 'r> de::Deserializer<'de> for De<'t, 'b, 'r> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self.node.kind {
            Kind::Bool => self.deserialize_bool(visitor),
            Kind::U8 => self.deserialize_u8(visitor),
            Kind::U16 => self.deserialize_u16(visitor),
            Kind::U32 => self.deserialize_u32(visitor),
            Kind::U64 => self.deserialize_u64(visitor),
            Kind::I8 => self.deserialize_i8(visitor),
            Kind::I16 => self.deserialize_i16(visitor),
            Kind::I32 => self.deserialize_i32(visitor),
            Kind::I64 => self.deserialize_i64(visitor),
            Kind::Float => self.deserialize_f32(visitor),
            Kind::Double => self.deserialize_f64(visitor),
            Kind::Char => self.deserialize_char(visitor),
            Kind::String => self.deserialize_string(visitor),
            Kind::Map => self.deserialize_map(visitor),
            Kind::Typeless => self.deserialize_seq(visitor),
            Kind::Empty | Kind::TypeMarker => visitor.visit_unit(),
            Kind::ArrayWrapper | Kind::Array => self.deserialize_seq(visitor),
            Kind::Struct => {
                let node = self.node;
                let rd = self.rd;
                let r = visitor.visit_map(StructAccess {
                    node,
                    rd: &mut *rd,
                    next: 0,
                    wanted: None,
                    pending: None,
                });
                if node.aligns() {
                    rd.align(4);
                }
                r
            }
            Kind::Referenced => Err(Error::Unsupported(format!(
                "managed references ({} `{}`)",
                self.node.type_name, self.node.name
            ))),
        }
    }

    primitive!(deserialize_bool, Bool, "bool", bool_, visit_bool);
    primitive!(deserialize_u8, U8, "UInt8", u8, visit_u8);
    primitive!(deserialize_u16, U16, "UInt16", u16_le, visit_u16);
    primitive!(deserialize_u32, U32, "UInt32", u32_le, visit_u32);
    primitive!(deserialize_u64, U64, "UInt64", u64_le, visit_u64);
    primitive!(deserialize_i8, I8, "SInt8", i8_, visit_i8);
    primitive!(deserialize_i16, I16, "SInt16", i16_le, visit_i16);
    primitive!(deserialize_i32, I32, "SInt32", i32_le, visit_i32);
    primitive!(deserialize_i64, I64, "SInt64", i64_le, visit_i64);
    primitive!(deserialize_f32, Float, "float", f32_le, visit_f32);
    primitive!(deserialize_f64, Double, "double", f64_le, visit_f64);

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if self.node.kind != Kind::Char {
            return Err(invalid_type(self.node, "char"));
        }
        let c = self.rd.u8(OBJECT)? as char;
        if self.node.aligns() {
            self.rd.align(4);
        }
        visitor.visit_char(c)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if self.node.kind != Kind::String {
            return Err(invalid_type(self.node, "string"));
        }
        let n = self.rd.u32_le(OBJECT)? as usize;
        let bytes = copy_of(self.rd.bytes(n, OBJECT)?, "string")?;
        // A string is always padded
        self.rd.align(4);
        match String::from_utf8(bytes) {
            Ok(s) => visitor.visit_string(s),
            Err(e) => visitor.visit_byte_buf(e.into_bytes()),
        }
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, _: V) -> Result<V::Value> {
        Err(invalid_type(self.node, "bytes (use a sequence of u8)"))
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, _: V) -> Result<V::Value> {
        Err(invalid_type(
            self.node,
            "a byte buffer (use a sequence of u8)",
        ))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(self, _: &'static str, _: V) -> Result<V::Value> {
        Err(invalid_type(self.node, "a unit struct"))
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if self.node.kind == Kind::Typeless {
            let n = self.rd.u32_le(OBJECT)? as usize;
            let bytes = self.rd.bytes(n, OBJECT)?;
            let r = visitor.visit_seq(ByteSeq {
                bytes: bytes.iter(),
            });
            if aligns_with_child(self.node) {
                self.rd.align(4);
            }
            return r;
        }
        let (array, item) = array_of(self.node)?;
        let n = read_count(self.rd, item.min_size, "array length")?;
        let mut access = ElementAccess {
            item,
            rd: &mut *self.rd,
            left: n,
        };
        let r = visitor.visit_seq(&mut access)?;
        if access.left != 0 {
            return Err(Error::Deserialize(format!(
                "the sequence `{}` has {n} elements but only {} were read",
                self.node.name,
                n - access.left
            )));
        }
        if self.node.aligns() || array.aligns() {
            self.rd.align(4);
        }
        Ok(r)
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value> {
        self.tuple(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: usize,
        visitor: V,
    ) -> Result<V::Value> {
        self.tuple(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if self.node.kind != Kind::Map {
            return Err(invalid_type(self.node, "a map"));
        }
        let (array, pair) = array_of(self.node)?;
        let [key, value] = match pair.children.as_slice() {
            [k, v] => [k, v],
            _ => {
                return Err(bad_tree(
                    self.node,
                    "a map whose pairs do not have two fields",
                ))
            }
        };
        let n = read_count(self.rd, pair.min_size, "map size")?;
        let mut access = PairAccess {
            key,
            value,
            rd: &mut *self.rd,
            left: n,
        };
        // When the visitor fails, its error is the one to report: the reader stands somewhere
        // inside an entry, and skipping from there would fail with an error about something else
        let r = visitor.visit_map(&mut access)?;
        // Entries the visitor left unread are skipped, so the data after the map stays in step
        let left = access.left;
        for _ in 0..left {
            skip(key, self.rd)?;
            skip(value, self.rd)?;
        }
        if self.node.aligns() || array.aligns() || pair.aligns() {
            self.rd.align(4);
        }
        Ok(r)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        let node = self.node;
        let rd = self.rd;
        let r = visitor.visit_map(StructAccess {
            node,
            rd: &mut *rd,
            next: 0,
            wanted: Some(fields),
            pending: None,
        });
        if node.aligns() {
            rd.align(4);
        }
        r
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        _: &'static [&'static str],
        _: V,
    ) -> Result<V::Value> {
        Err(Error::Deserialize(format!(
            "enum {name}: type trees have no enums; read the integer and convert it"
        )))
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, _: V) -> Result<V::Value> {
        Err(invalid_type(self.node, "an identifier"))
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        skip(self.node, self.rd)?;
        visitor.visit_unit()
    }
}

impl<'t, 'b, 'r> De<'t, 'b, 'r> {
    /// A tuple reads the children of the node in order. Children the tuple does not take are
    /// skipped.
    fn tuple<'de, V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let node = self.node;
        let rd = self.rd;
        let mut access = TupleAccess {
            node,
            rd: &mut *rd,
            next: 0,
        };
        let r = visitor.visit_seq(&mut access)?;
        let from = access.next;
        for c in &node.children[from..] {
            skip(c, rd)?;
        }
        if node.aligns() {
            rd.align(4);
        }
        Ok(r)
    }
}

struct ByteSeq<'a> {
    bytes: std::slice::Iter<'a, u8>,
}

impl<'de, 'a> SeqAccess<'de> for ByteSeq<'a> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        match self.bytes.next() {
            Some(&b) => seed.deserialize(b.into_deserializer()).map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.bytes.len())
    }
}

struct ElementAccess<'t, 'b, 'r> {
    item: &'t TypeTreeNode,
    rd: &'r mut Cursor<'b>,
    left: usize,
}

impl<'de, 't, 'b, 'r> SeqAccess<'de> for &mut ElementAccess<'t, 'b, 'r> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        self.rd.spend(1, "values to visit")?;
        seed.deserialize(De {
            node: self.item,
            rd: &mut *self.rd,
        })
        .map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.left)
    }
}

struct TupleAccess<'t, 'b, 'r> {
    node: &'t TypeTreeNode,
    rd: &'r mut Cursor<'b>,
    next: usize,
}

impl<'de, 't, 'b, 'r> SeqAccess<'de> for &mut TupleAccess<'t, 'b, 'r> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        let Some(child) = self.node.children.get(self.next) else {
            return Ok(None);
        };
        self.next += 1;
        self.rd.spend(1, "values to visit")?;
        seed.deserialize(De {
            node: child,
            rd: &mut *self.rd,
        })
        .map(Some)
    }
}

/// The children of a node as a `serde` map from field name to value.
struct StructAccess<'t, 'b, 'r> {
    node: &'t TypeTreeNode,
    rd: &'r mut Cursor<'b>,
    next: usize,
    /// The field names the Rust type asks for; `None` yields every child (for `deserialize_any`)
    wanted: Option<&'static [&'static str]>,
    pending: Option<&'t TypeTreeNode>,
}

impl<'de, 't, 'b, 'r> MapAccess<'de> for StructAccess<'t, 'b, 'r> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        while let Some(child) = self.node.children.get(self.next) {
            let index = self.next;
            self.next += 1;
            self.rd.spend(1, "values to visit")?;
            let take = match self.wanted {
                None => true,
                // The first child of that name is the field; a repeated name is skipped
                Some(fields) => {
                    fields.contains(&child.name.as_str())
                        && !self.node.children[..index]
                            .iter()
                            .any(|c| c.name == child.name)
                }
            };
            if take {
                self.pending = Some(child);
                return seed.deserialize(KeyDe { key: &child.name }).map(Some);
            }
            skip(child, self.rd)?;
        }
        Ok(None)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        let child = self
            .pending
            .take()
            .ok_or_else(|| Error::Deserialize("value requested before a key".to_string()))?;
        seed.deserialize(De {
            node: child,
            rd: &mut *self.rd,
        })
    }
}

/// A field name as the key of a map.
struct KeyDe<'a> {
    key: &'a str,
}

impl<'de, 'a> de::Deserializer<'de> for KeyDe<'a> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_str(self.key)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
        identifier ignored_any
    }
}

/// The entries of a `map` node.
struct PairAccess<'t, 'b, 'r> {
    key: &'t TypeTreeNode,
    value: &'t TypeTreeNode,
    rd: &'r mut Cursor<'b>,
    left: usize,
}

impl<'de, 't, 'b, 'r> MapAccess<'de> for &mut PairAccess<'t, 'b, 'r> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        self.rd.spend(1, "values to visit")?;
        seed.deserialize(MapKeyDe {
            node: self.key,
            rd: &mut *self.rd,
        })
        .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        seed.deserialize(De {
            node: self.value,
            rd: &mut *self.rd,
        })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.left)
    }
}

/// The key of a `map` entry. Keys are read as they are, except that a Rust type asking for a
/// string gets a `UInt32` key as its decimal text, so that maps keyed by hashes can be a
/// `HashMap<String, _>`.
struct MapKeyDe<'t, 'b, 'r> {
    node: &'t TypeTreeNode,
    rd: &'r mut Cursor<'b>,
}

impl<'de, 't, 'b, 'r> de::Deserializer<'de> for MapKeyDe<'t, 'b, 'r> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        De {
            node: self.node,
            rd: self.rd,
        }
        .deserialize_any(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self.node.kind {
            Kind::U32 => {
                let v = self.rd.u32_le(OBJECT)?;
                if self.node.aligns() {
                    self.rd.align(4);
                }
                visitor.visit_string(v.to_string())
            }
            Kind::String => de::Deserializer::deserialize_string(
                De {
                    node: self.node,
                    rd: self.rd,
                },
                visitor,
            ),
            _ => Err(invalid_type(self.node, "a string map key")),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char bytes byte_buf identifier
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
        ignored_any
    }
}
