//! Reads the asset bundles and serialized files of Unity 2018.4 and 2019.4.
//!
//! * [`bundle`] opens a UnityFS bundle (format version 6) and decompresses the files in it, whole
//!   or a range at a time.
//! * [`serialized`] parses a serialized file (format versions 17 and 21): its objects, its type
//!   table and the files it points into.
//! * [`de`] turns an object's bytes into any `serde` type through the type tree stored with the
//!   file, so that the layout of a class is never written down in code.
//!
//! The crate reads from slices and never panics on malformed input: every size in the data is
//! checked against what the data can hold before anything is allocated. Formats and the
//! evidence for them are in `docs/FORMATS.md`.
//!
//! ```
//! use serde::Deserialize;
//! use unity_bundle::{bundle::Bundle, serialized::SerializedFile};
//!
//! #[derive(Deserialize)]
//! #[allow(non_snake_case)]
//! struct GameObject {
//!     m_Name: String,
//! }
//!
//! # fn demo(bytes: &[u8]) -> unity_bundle::error::Result<()> {
//! let bundle = Bundle::new(bytes)?;
//! let entry = bundle.main_serialized_entry().expect("a serialized file");
//! let data = bundle.read_entry(entry)?;
//! let file = SerializedFile::parse(&data)?;
//! for info in file.objects().iter().filter(|o| o.class_id.0 == 1) {
//!     let go: GameObject = file.read(&data, info)?;
//!     println!("{}", go.m_Name);
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bundle;
pub mod class_id;
mod common_strings;
mod cursor;
pub mod de;
pub mod error;
mod lz4;
pub mod serialized;
pub mod typetree;

pub use bundle::Bundle;
pub use class_id::ClassId;
pub use error::{Error, Result};
pub use serialized::SerializedFile;
pub use typetree::TypeTreeNode;

// The code in the README is compiled by `cargo test`
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
