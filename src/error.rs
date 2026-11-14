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
    /// The memory for a table or a buffer could not be had. Its size had already been checked
    /// against the input, so this is the machine running out, not a claim the data cannot back.
    OutOfMemory {
        /// What the memory was for.
        what: &'static str,
        /// The bytes that were asked for (for the index of the objects by path id, the bytes of
        /// its entries: a hash table asks for more than that).
        bytes: u64,
    },
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
            Error::OutOfMemory { what, bytes } => {
                write!(f, "out of memory: {bytes} bytes for {what}")
            }
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

fn out_of_memory<T>(count: usize, what: &'static str) -> Error {
    Error::OutOfMemory {
        what,
        bytes: (count as u64).saturating_mul(std::mem::size_of::<T>() as u64),
    }
}

/// Makes room for exactly `additional` more elements. The caller has checked the count against
/// the input; when the memory cannot be had this is an error, where `Vec::with_capacity` would
/// end the process.
pub(crate) fn reserve<T>(v: &mut Vec<T>, additional: usize, what: &'static str) -> Result<()> {
    v.try_reserve_exact(additional)
        .map_err(|_| out_of_memory::<T>(additional, what))
}

/// Makes room for `additional` more elements of a vector that grows as the input is read (the
/// capacity may grow by more, as `Vec::push` would have it).
pub(crate) fn grow<T>(v: &mut Vec<T>, additional: usize, what: &'static str) -> Result<()> {
    v.try_reserve(additional)
        .map_err(|_| out_of_memory::<T>(v.len().saturating_add(additional), what))
}

/// A vector of `len` zero bytes.
pub(crate) fn zeroed(len: usize, what: &'static str) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    reserve(&mut v, len, what)?;
    v.resize(len, 0);
    Ok(v)
}

/// A copy of bytes of the input.
pub(crate) fn copy_of(bytes: &[u8], what: &'static str) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    reserve(&mut v, bytes.len(), what)?;
    v.extend_from_slice(bytes);
    Ok(v)
}
