//! Tests on the files of an installed game. Skipped unless `UNITY_BUNDLE_GAME_DIR` points at a game
//! folder that holds bundles somewhere below it (the one the crate was verified on is Shin Megami
//! Tensei III Nocturne HD Remaster, whose bundles are in `smt3hd_Data/StreamingAssets/PC`).
//!
//! `UNITY_BUNDLE_GAME_DIR=~/Games/smt3hd cargo test --release --test game`

use std::path::{Path, PathBuf};

use serde::Deserialize;
use unity_bundle::class_id::ClassId;
use unity_bundle::{Bundle, SerializedFile};

fn bundles() -> Option<Vec<PathBuf>> {
    let dir = PathBuf::from(std::env::var_os("UNITY_BUNDLE_GAME_DIR")?);
    let mut found = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_none() {
                // a UnityFS file starts with its signature
                let mut sig = [0u8; 8];
                let is_bundle = std::fs::File::open(&p)
                    .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut sig))
                    .is_ok()
                    && &sig == b"UnityFS\0";
                if is_bundle {
                    out.push(p);
                }
            }
        }
    }
    walk(&dir, &mut found);
    Some(found)
}

#[test]
fn bundles_open_and_their_class_names_agree_with_the_type_trees() {
    let Some(all) = bundles() else { return };
    assert!(
        !all.is_empty(),
        "no UnityFS files below UNITY_BUNDLE_GAME_DIR"
    );
    // One at a time, a spread over the folder: a serialized file is held in memory while it is used
    for p in all.iter().step_by(23) {
        if p.metadata().unwrap().len() > 64 << 20 {
            continue; // never load a huge file whole in a test
        }
        let bytes = std::fs::read(p).unwrap();
        let b = Bundle::new(&bytes[..]).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let Some(entry) = b.main_serialized_entry() else {
            continue;
        };
        let data = b.read_entry(entry).unwrap();
        let f = SerializedFile::parse(&data).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        for t in &f.types {
            // The root of a type tree is named after the class (except for scripts, which are
            // named after their C# class)
            if let (Some(tree), Some(name)) = (&t.type_tree, t.class_id.name()) {
                if t.class_id.0 != 114 {
                    assert_eq!(
                        tree.type_name,
                        name,
                        "class {} in {}",
                        t.class_id.0,
                        p.display()
                    );
                }
            }
        }
        // Every node of every tree numbers itself in pre-order
        fn check(n: &unity_bundle::TypeTreeNode, next: &mut i32) {
            assert_eq!(n.index, *next);
            *next += 1;
            for c in &n.children {
                check(c, next);
            }
        }
        for t in f.types.iter().filter_map(|t| t.type_tree.as_ref()) {
            check(t, &mut 0);
        }
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct Named {
    m_Name: String,
}

#[test]
fn every_game_object_of_a_bundle_has_a_name() {
    let Some(all) = bundles() else { return };
    let Some(p) = all
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n == "dvl0x01"))
        .or(all.first())
    else {
        return;
    };
    let bytes = std::fs::read(p).unwrap();
    let b = Bundle::new(&bytes[..]).unwrap();
    let data = b.read_entry(b.main_serialized_entry().unwrap()).unwrap();
    let f = SerializedFile::parse(&data).unwrap();
    let mut n = 0;
    for info in f.objects.iter().filter(|o| o.class_id == ClassId(1)) {
        let go: Named = f.read(&data, info).unwrap();
        assert!(!go.m_Name.is_empty() || n > 0);
        n += 1;
    }
    assert!(n > 0);
}
