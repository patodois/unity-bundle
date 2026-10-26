//! Prints what is in a bundle (or in a bare serialized file): the header, the blocks, the files,
//! the type table and the objects per class.
//!
//! ```text
//! cargo run --release --example inspect -- path/to/bundle            # the tables
//! cargo run --release --example inspect -- path/to/bundle --tree 28  # + the type tree of class 28
//! cargo run --release --example inspect -- path/to/bundle --hex 96   # + the first 96 bytes of the file
//! ```

use std::collections::BTreeMap;

use unity_bundle::bundle::Bundle;
use unity_bundle::class_id::ClassId;
use unity_bundle::SerializedFile;

fn hex(bytes: &[u8]) {
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let h: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let a: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("{:08x}  {:<47}  {a}", i * 16, h.join(" "));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: inspect FILE [--tree CLASS_ID] [--hex N]")?;
    let mut tree_of = None;
    let mut hex_n = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tree" => {
                tree_of = Some(
                    args.next()
                        .ok_or("--tree needs a class id")?
                        .parse::<i32>()?,
                )
            }
            "--hex" => {
                hex_n = Some(
                    args.next()
                        .ok_or("--hex needs a length")?
                        .parse::<usize>()?,
                )
            }
            other => return Err(format!("unknown option {other}").into()),
        }
    }
    let bytes = std::fs::read(&path)?;
    if let Some(n) = hex_n {
        hex(&bytes[..n.min(bytes.len())]);
    }

    let data = match Bundle::new(&bytes[..]) {
        Ok(b) => {
            let h = b.header();
            println!(
                "UnityFS version {}, player {}, written by Unity {}",
                h.format_version, h.player_version, h.unity_version
            );
            println!(
                "size {}, block info {} bytes at {} (-> {}), flags {:#x} ({:?})",
                h.size,
                h.block_info_compressed_size,
                h.block_info_offset,
                h.block_info_uncompressed_size,
                h.flags,
                h.block_info_compression()
            );
            let total: u64 = b.blocks().iter().map(|k| k.uncompressed_size as u64).sum();
            println!("{} blocks, {} bytes decompressed", b.blocks().len(), total);
            for e in b.entries() {
                println!(
                    "  file {:<40} offset {:>10} size {:>10} flags {:#x}",
                    e.path, e.offset, e.size, e.flags
                );
            }
            let Some(main) = b.main_serialized_entry() else {
                return Ok(());
            };
            b.read_entry(main)?
        }
        Err(e) => {
            println!("not a UnityFS bundle ({e}); trying a serialized file");
            bytes
        }
    };

    let f = SerializedFile::parse(&data)?;
    println!(
        "serialized file: format {}, Unity {}, platform {}, type trees {}, data at {}, {} types, {} objects, {} externals",
        f.format_version,
        f.unity_version,
        f.target_platform,
        f.has_type_tree,
        f.data_offset,
        f.types.len(),
        f.objects.len(),
        f.externals.len()
    );
    for (i, e) in f.externals.iter().enumerate() {
        println!("  external {}: {}", i + 1, e.path);
    }
    let mut per_class: BTreeMap<i32, usize> = BTreeMap::new();
    for o in &f.objects {
        *per_class.entry(o.class_id.0).or_default() += 1;
    }
    for (id, n) in per_class {
        println!(
            "  class {id:>10} {:<28} {n} objects",
            format!("{:?}", ClassId(id))
        );
    }
    if let Some(id) = tree_of {
        let t = f
            .types
            .iter()
            .find(|t| t.class_id.0 == id)
            .ok_or("no such class in the type table")?;
        match &t.type_tree {
            Some(tree) => print!("{}", tree.dump()),
            None => println!("(no type tree)"),
        }
    }
    Ok(())
}
