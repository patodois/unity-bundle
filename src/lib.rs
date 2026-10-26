//! Reads the asset bundles of Unity 2018.4 and 2019.4: UnityFS bundles (container format version 6),
//! their block table and directory, LZ4 blocks, and byte ranges of the files inside.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bundle;
mod cursor;
pub mod error;
mod lz4;

pub use bundle::Bundle;
pub use error::{Error, Result};
