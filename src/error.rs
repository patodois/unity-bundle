//! The error type of the crate.

use std::fmt;

/// What went wrong while reading a bundle, a serialized file or an object.
///
/// Every failure on malformed input is one of these; the readers never panic and never allocate
/// more than the input can justify.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The input ends before a structure is complete.
    Truncated {
        /// What was being read.
        what: &'static str,
    },
    /// The input is not well formed.
    Invalid(String),
    /// The input is well formed but uses a feature this crate does not read (another bundle
    /// format version, a compression method, a big-endian file, ...).
    Unsupported(String),
    /// A size in the input is larger than the crate accepts or than the input can hold.
    TooLarge {
        /// What the size describes.
        what: &'static str,
        /// The size found in the input.
        value: u64,
        /// The most that is accepted at that point.
        limit: u64,
    },
    /// A compressed block does not decompress to what the block table says.
    Decompress(String),
    /// A path id that the serialized file does not hold.
    NoObject(i64),
    /// The object's type has no type tree (the file was written without type trees).
    NoTypeTree,
    /// A failure while deserializing an object into a Rust type, with the type tree in hand.
    Deserialize(String),
}

/// `Result` with this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated { what } => write!(f, "unexpected end of data while reading {what}"),
            Error::Invalid(m) => write!(f, "invalid data: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error::TooLarge { what, value, limit } => {
                write!(f, "{what} of {value} exceeds the limit of {limit}")
            }
            Error::Decompress(m) => write!(f, "decompression failed: {m}"),
            Error::NoObject(id) => write!(f, "no object with path id {id}"),
            Error::NoTypeTree => f.write_str("the object's type has no type tree"),
            Error::Deserialize(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Deserialize(msg.to_string())
    }
}

pub(crate) fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}
