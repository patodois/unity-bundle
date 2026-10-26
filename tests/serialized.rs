mod common;

use common::*;
use serde::Deserialize;
use unity_bundle::class_id::ClassId;
use unity_bundle::serialized::{SerializedFile, FORMAT_VERSION, FORMAT_VERSION_2018_4};
use unity_bundle::Error;

#[derive(Debug, Deserialize, PartialEq)]
#[allow(non_snake_case)]
struct Named {
    m_Name: String,
    m_Value: i32,
}

fn named_tree() -> N {
    n(
        "Named",
        "Base",
        vec![
            string("m_Name"),
            prim("int", "m_Value"),
            prim("float", "m_Unused"),
        ],
    )
}

fn named_data(name: &str, value: i32) -> Vec<u8> {
    let mut w = W::default();
    w.string(name).i32(value).f32(1.5);
    w.0
}

fn file(version: u32) -> Vec<u8> {
    serialized_file(
        version,
        &[
            TypeSpec {
                class_id: 114,
                tree: Some(named_tree()),
            },
            TypeSpec {
                class_id: 1,
                tree: Some(named_tree()),
            },
        ],
        &[
            ObjectSpec {
                path_id: 5,
                type_index: 0,
                data: named_data("first", 11),
            },
            ObjectSpec {
                path_id: -9,
                type_index: 1,
                data: named_data("second one", -22),
            },
            ObjectSpec {
                path_id: 1 << 40,
                type_index: 1,
                data: named_data("", 33),
            },
        ],
        &["archive:/CAB-x/CAB-x", "Resources/unity_builtin_extra"],
        true,
    )
}

#[test]
fn parses_both_supported_versions() {
    for version in [FORMAT_VERSION, FORMAT_VERSION_2018_4] {
        let data = file(version);
        let f = SerializedFile::parse(&data).unwrap();
        assert_eq!(f.format_version, version);
        assert_eq!(f.unity_version, "2019.4.9f1");
        assert_eq!(f.target_platform, 19);
        assert!(f.has_type_tree);
        assert_eq!(f.types.len(), 2);
        assert_eq!(f.types[0].class_id, ClassId(114));
        assert_eq!(
            f.types[0].script_id, [7; 16],
            "a MonoBehaviour type carries a script hash"
        );
        assert_eq!(f.types[1].script_id, [0; 16]);
        assert_eq!(f.types[1].old_type_hash, [9; 16]);
        assert_eq!(f.objects().len(), 3);
        assert_eq!(f.objects()[1].class_id, ClassId(1));
        assert_eq!(f.objects()[1].class_id.name(), Some("GameObject"));
        assert_eq!(f.externals.len(), 2);
        assert_eq!(f.externals[0].path, "archive:/CAB-x/CAB-x");
        assert_eq!(f.user_information, "");
        let tree = f.types[1].type_tree.as_ref().unwrap();
        assert_eq!(tree.type_name(), "Named");
        assert_eq!(tree.children().len(), 3);
        assert_eq!(tree.children()[0].children()[0].type_name(), "Array");
        assert_eq!(tree.dump().lines().next(), Some("Named Base"));
        assert!(tree
            .dump()
            .contains("\n  string m_Name\n    Array Array\n      int size\n      char data\n"));
    }
}

#[test]
fn reads_objects_through_their_type_trees() {
    for version in [FORMAT_VERSION, FORMAT_VERSION_2018_4] {
        let data = file(version);
        let f = SerializedFile::parse(&data).unwrap();
        let a: Named = f.read_path_id(&data, 5).unwrap();
        assert_eq!(
            a,
            Named {
                m_Name: "first".into(),
                m_Value: 11
            }
        );
        let b: Named = f.read_path_id(&data, -9).unwrap();
        assert_eq!(
            b,
            Named {
                m_Name: "second one".into(),
                m_Value: -22
            }
        );
        let c: Named = f.read_path_id(&data, 1 << 40).unwrap();
        assert_eq!(
            c,
            Named {
                m_Name: String::new(),
                m_Value: 33
            }
        );
        assert!(matches!(
            f.read_path_id::<Named>(&data, 6),
            Err(Error::NoObject(6))
        ));
        // raw bytes are the object's own
        let info = f.object(5).unwrap();
        assert_eq!(f.raw(&data, info).unwrap(), &named_data("first", 11)[..]);
        assert_eq!(info.offset % 8, 0);
    }
}

#[test]
fn a_file_without_type_trees_cannot_be_read_into_types() {
    let data = serialized_file(
        FORMAT_VERSION,
        &[TypeSpec {
            class_id: 28,
            tree: None,
        }],
        &[ObjectSpec {
            path_id: 1,
            type_index: 0,
            data: vec![1, 2, 3, 4],
        }],
        &[],
        false,
    );
    let f = SerializedFile::parse(&data).unwrap();
    assert!(!f.has_type_tree);
    assert_eq!(f.objects().len(), 1);
    assert!(f.types[0].type_tree.is_none());
    assert!(matches!(
        f.read_path_id::<Named>(&data, 1),
        Err(Error::NoTypeTree)
    ));
    assert_eq!(f.raw(&data, f.object(1).unwrap()).unwrap(), &[1, 2, 3, 4]);
}

#[test]
fn other_versions_and_byte_orders_are_refused() {
    let mut data = file(FORMAT_VERSION);
    data[8..12].copy_from_slice(&22u32.to_be_bytes());
    assert!(matches!(
        SerializedFile::parse(&data),
        Err(Error::Unsupported(_))
    ));
    data[8..12].copy_from_slice(&19u32.to_be_bytes());
    assert!(matches!(
        SerializedFile::parse(&data),
        Err(Error::Unsupported(_))
    ));
    let mut data = file(FORMAT_VERSION);
    data[16] = 1; // big-endian data
    assert!(matches!(
        SerializedFile::parse(&data),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn every_prefix_of_a_file_is_an_error_not_a_panic() {
    for version in [FORMAT_VERSION, FORMAT_VERSION_2018_4] {
        let data = file(version);
        let f = SerializedFile::parse(&data).unwrap();
        let metadata_end = 20 + f.metadata_size as usize;
        for n in 0..metadata_end {
            assert!(
                SerializedFile::parse(&data[..n]).is_err(),
                "version {version}, prefix of {n}"
            );
        }
        // Cut inside the object data: the tables parse, the objects do not read
        let cut = &data[..data.len() - 3];
        let f2 = SerializedFile::parse(cut).unwrap();
        assert!(
            f2.read_path_id::<Named>(cut, 1 << 40).is_err()
                || f2.raw(cut, f2.object(1 << 40).unwrap()).is_err()
        );
    }
}

#[test]
fn an_object_that_lies_outside_the_data_is_an_error() {
    let data = file(FORMAT_VERSION);
    let f = SerializedFile::parse(&data).unwrap();
    let mut info = f.objects()[0];
    info.offset = u64::MAX;
    assert!(f.raw(&data, &info).is_err());
    info.offset = data.len() as u64 - 2;
    info.size = 100;
    assert!(f.raw(&data, &info).is_err());
}

#[test]
fn a_repeated_path_id_resolves_to_the_last_object() {
    let data = serialized_file(
        FORMAT_VERSION,
        &[TypeSpec {
            class_id: 1,
            tree: Some(named_tree()),
        }],
        &[
            ObjectSpec {
                path_id: 3,
                type_index: 0,
                data: named_data("a", 1),
            },
            ObjectSpec {
                path_id: 3,
                type_index: 0,
                data: named_data("b", 2),
            },
        ],
        &[],
        true,
    );
    let f = SerializedFile::parse(&data).unwrap();
    assert_eq!(f.read_path_id::<Named>(&data, 3).unwrap().m_Value, 2);
}

#[test]
fn class_ids_have_names() {
    assert_eq!(ClassId(28).name(), Some("Texture2D"));
    assert_eq!(ClassId(137).name(), Some("SkinnedMeshRenderer"));
    assert_eq!(ClassId(1773428102).name(), Some("ParentConstraint"));
    assert_eq!(ClassId(123456).name(), None);
    assert_eq!(format!("{:?}", ClassId(1)), "GameObject");
    assert_eq!(format!("{:?}", ClassId(-5)), "ClassId(-5)");
}
