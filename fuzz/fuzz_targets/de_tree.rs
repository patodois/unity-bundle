#![no_main]
//! `de::from_slice` with a type tree built through the public `TypeTreeNode::new` (so trees the
//! blob reader would never produce are tried too) and arbitrary data.
use std::collections::{BTreeMap, HashMap};

use libfuzzer_sys::arbitrary::{Arbitrary, Result, Unstructured};
use libfuzzer_sys::fuzz_target;
use serde::de::IgnoredAny;
use unity_bundle::de::from_slice;
use unity_bundle::TypeTreeNode;
use unity_bundle_fuzz::Wanted;

const TYPES: &[&str] = &[
    "bool",
    "UInt8",
    "UInt16",
    "unsigned short",
    "UInt32",
    "unsigned int",
    "Type*",
    "UInt64",
    "unsigned long long",
    "FileSize",
    "SInt8",
    "SInt16",
    "short",
    "SInt32",
    "int",
    "SInt64",
    "long long",
    "float",
    "double",
    "char",
    "string",
    "map",
    "Type",
    "Array",
    "TypelessData",
    "ReferencedObject",
    "vector",
    "set",
    "pair",
    "PPtr<Object>",
    "Wanted",
    "Thing",
];
const NAMES: &[&str] = &[
    "Base", "m_Name", "m_Value", "m_List", "m_Junk", "data", "size", "first", "second", "", "x",
];

struct Spec {
    ty: u8,
    name: u8,
    align: bool,
    children: Vec<Spec>,
}

impl<'a> Arbitrary<'a> for Spec {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Spec> {
        Ok(Spec {
            ty: u.arbitrary()?,
            name: u.arbitrary()?,
            align: u.arbitrary()?,
            children: u.arbitrary()?,
        })
    }
}

fn build(s: &Spec, counter: &mut i32) -> TypeTreeNode {
    let index = *counter;
    *counter += 1;
    let children = s.children.iter().map(|c| build(c, counter)).collect();
    TypeTreeNode::new(
        TYPES[s.ty as usize % TYPES.len()].into(),
        NAMES[s.name as usize % NAMES.len()].into(),
        1,
        0,
        -1,
        index,
        if s.align { 1 << 14 } else { 0 },
        0,
        children,
    )
}

fuzz_target!(|input: &[u8]| {
    let mut u = Unstructured::new(input);
    let Ok(spec) = Spec::arbitrary(&mut u) else {
        return;
    };
    let data = u.take_rest();
    let tree = build(&spec, &mut 0);
    unity_bundle_fuzz::bounded(input.len(), || {
        let _ = tree.dump();
        let _ = from_slice::<IgnoredAny>(data, &tree);
        let _ = from_slice::<serde_json::Value>(data, &tree);
        let _ = from_slice::<Wanted>(data, &tree);
        let _ = from_slice::<Vec<IgnoredAny>>(data, &tree);
        let _ = from_slice::<(i32, String, Vec<u8>)>(data, &tree);
        let _ = from_slice::<HashMap<String, IgnoredAny>>(data, &tree);
        let _ = from_slice::<BTreeMap<u32, serde_json::Value>>(data, &tree);
    });
});
