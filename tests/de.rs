mod common;

use std::collections::HashMap;

use common::*;
use serde::de::IgnoredAny;
use serde::Deserialize;
use unity_bundle::de::from_slice;
use unity_bundle::{Error, TypeTreeNode};

fn node(n: &N, counter: &mut i32) -> TypeTreeNode {
    let index = *counter;
    *counter += 1;
    let children = n.children.iter().map(|c| node(c, counter)).collect();
    TypeTreeNode::new(
        n.ty.into(),
        n.name.into(),
        1,
        0,
        -1,
        index,
        n.meta,
        0,
        children,
    )
}

fn read<T: serde::de::DeserializeOwned>(tree: &N, data: &[u8]) -> Result<T, Error> {
    from_slice(data, &node(tree, &mut 0))
}

#[derive(Debug, Deserialize, PartialEq)]
#[allow(non_snake_case)]
struct Prims {
    a: bool,
    b: u8,
    c: i8,
    d: u16,
    e: i16,
    f: u32,
    g: i32,
    h: u64,
    i: i64,
    j: f32,
    k: f64,
    l: char,
}

fn prims_tree() -> N {
    n(
        "Prims",
        "Base",
        vec![
            na("bool", "a", vec![]),
            prim("UInt8", "b"),
            na("SInt8", "c", vec![]),
            prim("UInt16", "d"),
            na("short", "e", vec![]),
            prim("unsigned int", "f"),
            prim("int", "g"),
            prim("FileSize", "h"),
            prim("long long", "i"),
            prim("float", "j"),
            prim("double", "k"),
            prim("char", "l"),
        ],
    )
}

#[test]
fn primitives_and_the_padding_after_flagged_ones() {
    let mut w = W::default();
    w.u8(1).align4(); // bool with the align flag
    w.u8(200);
    w.u8(0xFE).align4(); // SInt8, aligned
    w.u16(65000);
    w.i16(-2).align4();
    w.u32(4_000_000_000).i32(-5).u64(1 << 50).i64(-(1 << 50));
    w.f32(2.5).bytes(&(-0.125f64).to_le_bytes()).u8(b'z');
    let p: Prims = read(&prims_tree(), &w.0).unwrap();
    assert_eq!(
        p,
        Prims {
            a: true,
            b: 200,
            c: -2,
            d: 65000,
            e: -2,
            f: 4_000_000_000,
            g: -5,
            h: 1 << 50,
            i: -(1 << 50),
            j: 2.5,
            k: -0.125,
            l: 'z'
        }
    );
}

#[test]
fn a_primitive_is_read_as_its_own_type_only() {
    let tree = n("S", "Base", vec![prim("float", "x")]);
    let data = 1.0f32.to_le_bytes();
    #[derive(Deserialize, Debug)]
    struct AsInt {
        #[allow(dead_code)]
        x: i32,
    }
    #[derive(Deserialize, Debug)]
    struct AsFloat {
        x: f32,
    }
    let e = read::<AsInt>(&tree, &data).unwrap_err();
    assert!(e.to_string().contains("float"), "{e}");
    assert_eq!(read::<AsFloat>(&tree, &data).unwrap().x, 1.0);
}

#[test]
fn strings_with_padding_and_bytes_that_are_not_utf8() {
    #[derive(Deserialize, Debug)]
    struct S {
        a: String,
        b: String,
        n: i32,
    }
    let tree = n(
        "S",
        "Base",
        vec![string("a"), string("b"), prim("int", "n")],
    );
    let mut w = W::default();
    w.string("héllo").string("").i32(77);
    let s: S = read(&tree, &w.0).unwrap();
    assert_eq!((s.a.as_str(), s.b.as_str(), s.n), ("héllo", "", 77));

    // A string that is not UTF-8 can be read as bytes (the visitor gets visit_byte_buf)
    #[derive(Deserialize, Debug)]
    struct B {
        a: Bytes,
    }
    struct Bytes(Vec<u8>);
    impl std::fmt::Debug for Bytes {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }
    impl<'de> Deserialize<'de> for Bytes {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> serde::de::Visitor<'de> for V {
                type Value = Bytes;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("bytes")
                }
                fn visit_str<E>(self, v: &str) -> Result<Bytes, E> {
                    Ok(Bytes(v.as_bytes().to_vec()))
                }
                fn visit_string<E>(self, v: String) -> Result<Bytes, E> {
                    Ok(Bytes(v.into_bytes()))
                }
                fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Bytes, E> {
                    Ok(Bytes(v))
                }
            }
            d.deserialize_string(V)
        }
    }
    let tree = n("S", "Base", vec![string("a")]);
    let mut w = W::default();
    w.u32(3).bytes(&[0xFF, 0xFE, 0x41]).align4();
    assert_eq!(read::<B>(&tree, &w.0).unwrap().a.0, vec![0xFF, 0xFE, 0x41]);
    // ... and as a String it is an error
    assert!(read::<S>(
        &n(
            "S",
            "Base",
            vec![string("a"), string("b"), prim("int", "n")]
        ),
        &w.0
    )
    .is_err());
}

#[test]
fn vectors_of_primitives_structs_and_vectors() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct P {
        x: f32,
        y: f32,
    }
    #[derive(Deserialize, Debug, PartialEq)]
    #[allow(non_snake_case)]
    struct V {
        ints: Vec<i32>,
        pts: Vec<P>,
        nested: Vec<Vec<u8>>,
        after: u32,
    }
    let tree = n(
        "V",
        "Base",
        vec![
            vector("ints", prim("int", "")),
            vector(
                "pts",
                n("P", "", vec![prim("float", "x"), prim("float", "y")]),
            ),
            vector("nested", vector("", prim("UInt8", ""))),
            prim("unsigned int", "after"),
        ],
    );
    let mut w = W::default();
    w.u32(3).i32(1).i32(2).i32(3);
    w.u32(2).f32(0.5).f32(1.5).f32(-1.0).f32(-2.0);
    w.u32(2);
    w.u32(3).bytes(&[7, 8, 9]).align4();
    w.u32(0);
    w.align4();
    w.u32(123);
    let v: V = read(&tree, &w.0).unwrap();
    assert_eq!(
        v,
        V {
            ints: vec![1, 2, 3],
            pts: vec![P { x: 0.5, y: 1.5 }, P { x: -1.0, y: -2.0 }],
            nested: vec![vec![7, 8, 9], vec![]],
            after: 123
        }
    );
}

#[test]
fn padding_after_an_array_of_bytes_when_the_array_node_says_so() {
    // `vector` -> `Array` (aligned) of UInt8: 3 bytes then 1 byte of padding
    #[derive(Deserialize, Debug)]
    struct S {
        bytes: Vec<u8>,
        after: i32,
    }
    let tree = n(
        "S",
        "Base",
        vec![vector("bytes", prim("UInt8", "")), prim("int", "after")],
    );
    let mut w = W::default();
    w.u32(3).bytes(&[1, 2, 3]).align4().i32(9);
    let s: S = read(&tree, &w.0).unwrap();
    assert_eq!((s.bytes, s.after), (vec![1, 2, 3], 9));
}

#[test]
fn maps_as_sequences_of_pairs_and_as_maps() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct Tex {
        id: i32,
    }
    #[derive(Deserialize, Debug)]
    struct M {
        pairs: Vec<(String, Tex)>,
        by_hash: HashMap<String, f32>,
        by_name: HashMap<String, f32>,
    }
    let tree = n(
        "M",
        "Base",
        vec![
            map("pairs", string(""), n("Tex", "", vec![prim("int", "id")])),
            map("by_hash", prim("unsigned int", ""), prim("float", "")),
            map("by_name", string(""), prim("float", "")),
        ],
    );
    let mut w = W::default();
    w.u32(2).string("_MainTex").i32(5).string("_Bump").i32(6);
    w.u32(2).u32(4096).f32(1.0).u32(7).f32(2.0);
    w.u32(1).string("k").f32(3.0);
    let m: M = read(&tree, &w.0).unwrap();
    assert_eq!(
        m.pairs,
        vec![
            ("_MainTex".to_string(), Tex { id: 5 }),
            ("_Bump".to_string(), Tex { id: 6 })
        ]
    );
    assert_eq!(m.by_hash["4096"], 1.0);
    assert_eq!(m.by_hash["7"], 2.0);
    assert_eq!(m.by_name["k"], 3.0);
}

#[test]
fn a_map_visitor_that_fails_reports_its_own_error() {
    // A map of string to float read as a map of string to i32: the first value is refused. What
    // is left of the map (from inside the first entry) must not be skipped, since the error of
    // that skip would replace the one that says what is wrong.
    let tree = map("m", string(""), prim("float", ""));
    let mut w = W::default();
    w.u32(3);
    w.string("a")
        .f32(1.0)
        .string("b")
        .f32(2.0)
        .string("c")
        .f32(3.0);
    let e = read::<HashMap<String, i32>>(&tree, &w.0).unwrap_err();
    assert_eq!(
        e.to_string(),
        "invalid type: float `second`, expected SInt32"
    );
    // read as what it is, the same data is fine
    let m: HashMap<String, f32> = read(&tree, &w.0).unwrap();
    assert_eq!(m.len(), 3);
}

#[test]
fn tuples_take_the_children_in_order() {
    #[derive(Deserialize, Debug)]
    struct T {
        v: (f32, f32, f32),
        two: (f32, f32),
        after: i32,
    }
    let vec3 = |name| {
        n(
            "Vector3f",
            name,
            vec![prim("float", "x"), prim("float", "y"), prim("float", "z")],
        )
    };
    let tree = n(
        "T",
        "Base",
        vec![vec3("v"), vec3("two"), prim("int", "after")],
    );
    let mut w = W::default();
    w.f32(1.0)
        .f32(2.0)
        .f32(3.0)
        .f32(4.0)
        .f32(5.0)
        .f32(6.0)
        .i32(8);
    let t: T = read(&tree, &w.0).unwrap();
    assert_eq!(t.v, (1.0, 2.0, 3.0));
    // A tuple that takes fewer children than there are leaves the rest skipped, not unread
    assert_eq!(t.two, (4.0, 5.0));
    assert_eq!(t.after, 8);
}

#[test]
fn typeless_data_is_a_sequence_of_bytes_and_pads_when_flagged() {
    #[derive(Deserialize, Debug)]
    struct S {
        data: Vec<u8>,
        after: i32,
    }
    let mut padded = typeless("data");
    padded.meta = ALIGN;
    let tree = n("S", "Base", vec![padded, prim("int", "after")]);
    let mut w = W::default();
    w.u32(5).bytes(&[1, 2, 3, 4, 5]).align4().i32(-1);
    let s: S = read(&tree, &w.0).unwrap();
    assert_eq!((s.data.len(), s.after), (5, -1));
    // without the flag nothing is skipped
    let tree = n("S", "Base", vec![typeless("data"), prim("int", "after")]);
    let mut w = W::default();
    w.u32(4).bytes(&[1, 2, 3, 4]).i32(-1);
    let s: S = read(&tree, &w.0).unwrap();
    assert_eq!((s.data, s.after), (vec![1, 2, 3, 4], -1));
}

#[test]
fn fields_are_matched_by_name_missing_ones_default_and_unknown_ones_are_skipped() {
    #[derive(Deserialize, Debug)]
    struct Want {
        wanted: i32,
        #[serde(default)]
        absent: i32,
        last: String,
    }
    let tree = n(
        "Want",
        "Base",
        vec![
            string("junk1"),
            vector("junk2", n("P", "", vec![string("s"), prim("float", "f")])),
            map("junk3", string(""), vector("", prim("int", ""))),
            typeless("junk4"),
            pptr("junk5"),
            prim("int", "wanted"),
            prim("bool", "junk6"),
            string("last"),
        ],
    );
    let mut w = W::default();
    w.string("skip me");
    w.u32(2).string("a").f32(1.0).string("bc").f32(2.0);
    w.u32(1).string("k").u32(2).i32(1).i32(2);
    w.u32(3).bytes(&[1, 2, 3]);
    w.i32(0).i64(5);
    w.i32(42);
    w.u8(1);
    w.string("done");
    let r: Want = read(&tree, &w.0).unwrap();
    assert_eq!((r.wanted, r.absent, r.last.as_str()), (42, 0, "done"));

    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct NeedsMissing {
        not_there: i32,
    }
    let e = read::<NeedsMissing>(&tree, &w.0).unwrap_err();
    assert!(e.to_string().contains("not_there"), "{e}");
}

#[test]
fn ignored_any_skips_whole_values() {
    #[derive(Deserialize, Debug)]
    struct S {
        #[allow(dead_code)]
        junk: Vec<IgnoredAny>,
        keep: i32,
    }
    let tree = n(
        "S",
        "Base",
        vec![
            vector("junk", n("P", "", vec![string("s"), prim("int", "i")])),
            prim("int", "keep"),
        ],
    );
    let mut w = W::default();
    w.u32(2).string("x").i32(1).string("yz").i32(2).i32(99);
    assert_eq!(read::<S>(&tree, &w.0).unwrap().keep, 99);
}

#[test]
fn option_unit_and_newtype() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct Id(i32);
    #[derive(Deserialize, Debug)]
    struct S {
        o: Option<i32>,
        id: Id,
        e: (),
    }
    let tree = n(
        "S",
        "Base",
        vec![prim("int", "o"), prim("int", "id"), n("Empty", "e", vec![])],
    );
    let mut w = W::default();
    w.i32(1).i32(2);
    let s: S = read(&tree, &w.0).unwrap();
    assert_eq!((s.o, s.id), (Some(1), Id(2)));
    assert_eq!(s.e, ());
}

#[test]
fn enums_and_byte_buffers_are_refused() {
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    enum E {
        A,
    }
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct S {
        e: E,
    }
    let tree = n("S", "Base", vec![prim("int", "e")]);
    let e = read::<S>(&tree, &[0, 0, 0, 0]).unwrap_err();
    assert!(e.to_string().contains("enum"), "{e}");
}

#[test]
fn counts_that_the_data_cannot_hold_are_refused_before_anything_is_reserved() {
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct S {
        v: Vec<u32>,
    }
    let tree = n("S", "Base", vec![vector("v", prim("unsigned int", ""))]);
    for count in [u32::MAX, 1 << 30, 5, 3] {
        let mut w = W::default();
        w.u32(count).u32(1).u32(2);
        let r = read::<S>(&tree, &w.0);
        match count {
            3 => {
                // 3 elements of 4 bytes need 12 bytes and only 8 follow
                assert!(matches!(r, Err(Error::TooLarge { .. })), "{r:?}");
            }
            _ => assert!(matches!(r, Err(Error::TooLarge { .. })), "{count}: {r:?}"),
        }
    }

    // elements that take no bytes cannot be used to loop for long either
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Z {
        v: Vec<IgnoredAny>,
    }
    let tree = n("Z", "Base", vec![vector("v", n("Nothing", "", vec![]))]);
    let mut w = W::default();
    w.u32(u32::MAX);
    assert!(matches!(
        read::<Z>(&tree, &w.0),
        Err(Error::TooLarge { .. })
    ));

    // strings, typeless data, maps
    let mut w = W::default();
    w.u32(u32::MAX).bytes(b"abc");
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct St {
        s: String,
    }
    assert!(read::<St>(&n("S", "Base", vec![string("s")]), &w.0).is_err());
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Ty {
        s: Vec<u8>,
    }
    assert!(read::<Ty>(&n("S", "Base", vec![typeless("s")]), &w.0).is_err());
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Mp {
        m: Vec<(String, i32)>,
    }
    assert!(read::<Mp>(
        &n("S", "Base", vec![map("m", string(""), prim("int", ""))]),
        &w.0
    )
    .is_err());
}

#[test]
fn reading_past_the_end_is_an_error() {
    let tree = prims_tree();
    let mut w = W::default();
    w.u8(1).align4().u8(1);
    for n in 0..w.0.len() {
        assert!(read::<Prims>(&tree, &w.0[..n]).is_err());
    }
}

#[test]
fn managed_references_are_not_read() {
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct S {
        keep: i32,
    }
    let tree = n(
        "S",
        "Base",
        vec![
            prim("int", "keep"),
            n(
                "ManagedReferencesRegistry",
                "references",
                vec![prim("int", "version")],
            ),
        ],
    );
    let mut w = W::default();
    w.i32(1).i32(2);
    let e = read::<S>(&tree, &w.0).unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)), "{e}");
}

#[test]
fn generic_values_through_deserialize_any() {
    let tree = n(
        "S",
        "Base",
        vec![
            string("name"),
            prim("int", "n"),
            vector("list", prim("float", "")),
            map("m", string(""), prim("int", "")),
            typeless("blob"),
            pptr("ptr"),
            prim("bool", "flag"),
        ],
    );
    let mut w = W::default();
    w.string("x").i32(3);
    w.u32(2).f32(0.5).f32(1.5);
    w.u32(1).string("k").i32(4);
    w.u32(2).bytes(&[9, 8]);
    w.i32(0).i64(77);
    w.u8(1);
    let v: serde_json::Value = read(&tree, &w.0).unwrap();
    assert_eq!(
        v,
        serde_json::json!({
            "name": "x",
            "n": 3,
            "list": [0.5, 1.5],
            "m": {"k": 4},
            "blob": [9, 8],
            "ptr": {"m_FileID": 0, "m_PathID": 77},
            "flag": true,
        })
    );
}

/// A `string` node and its `Array` child, neither with the align flag
fn unflagged_string(name: &'static str) -> N {
    n(
        "string",
        name,
        vec![n(
            "Array",
            "Array",
            vec![prim("int", "size"), prim("char", "data")],
        )],
    )
}

#[test]
fn strings_are_padded_even_when_no_node_says_so() {
    #[derive(Deserialize, Debug)]
    struct S {
        a: String,
        after: i32,
    }
    #[derive(Deserialize, Debug)]
    struct After {
        after: i32,
    }
    let tree = n(
        "S",
        "Base",
        vec![unflagged_string("a"), prim("int", "after")],
    );
    let mut w = W::default();
    w.string("ab").i32(7); // 4 + 2 bytes, 2 of padding
    let s: S = read(&tree, &w.0).unwrap();
    assert_eq!((s.a.as_str(), s.after), ("ab", 7));
    // and when the string is skipped
    assert_eq!(read::<After>(&tree, &w.0).unwrap().after, 7);
}

#[test]
fn what_serde_features_do_with_type_trees() {
    use std::collections::HashMap;
    let tree = n("S", "Base", vec![prim("int", "a"), prim("int", "b")]);
    let data = [1, 0, 0, 0, 2, 0, 0, 0];

    // flatten and a map read from a struct node ask for a map; a struct node is not a `map` node
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Flat {
        #[serde(flatten)]
        rest: HashMap<String, i32>,
    }
    assert!(read::<Flat>(&tree, &data).is_err());
    assert!(read::<HashMap<String, i32>>(&tree, &data).is_err());

    // fields the type does not name are skipped before serde could object to them
    #[derive(Deserialize, Debug)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        a: i32,
    }
    assert_eq!(read::<Strict>(&tree, &data).unwrap().a, 1);

    // `()` reads nothing, and an Option is always Some
    assert!(read::<()>(&tree, &data).is_ok());
    assert_eq!(
        read::<Option<(i32, i32)>>(&tree, &data).unwrap(),
        Some((1, 2))
    );
}

#[test]
fn a_tree_cannot_ask_for_more_visits_than_the_object_is_long() {
    // each of the 40 bytes can stand for a handful of values, not for millions
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct S {
        v: Vec<Vec<IgnoredAny>>,
    }
    let tree = n(
        "S",
        "Base",
        vec![vector("v", vector("", n("Nothing", "", vec![])))],
    );
    let mut w = W::default();
    w.u32(8);
    for k in 0..8u32 {
        w.u32(4 * (8 - k - 1));
    }
    assert!(read::<S>(&tree, &w.0).is_ok());
    // 10,000 outer elements with an inner count each of everything that remains
    let mut w = W::default();
    w.u32(10_000);
    for k in 0..10_000u32 {
        w.u32(4 * (10_000 - k - 1));
    }
    // The error states the budget (16 for each of the 40,004 bytes, and 1,024) and the visit
    // that went over it, not the length of the object
    let e = read::<S>(&tree, &w.0).unwrap_err();
    assert_eq!(
        e,
        Error::TooLarge {
            what: "values to visit",
            value: 40_004 * 16 + 1024 + 1,
            limit: 40_004 * 16 + 1024,
        }
    );
    assert_eq!(
        e.to_string(),
        "values to visit of 641089 exceeds the limit of 641088"
    );
}
