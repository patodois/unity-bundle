//! UnityFS bundles: the header, the block table, the file directory, and reading files out of the
//! compressed blocks.
//!
//! A bundle is a header, a compressed table (the *block info*) and then the blocks. The table
//! lists the blocks (sizes and compression) and the files (offset, size, flags and name in the
//! concatenation of the decompressed blocks). See `docs/FORMATS.md` for the layout, with the
//! evidence from the game files.

use crate::cursor::Cursor;
use crate::error::{invalid, Error, Result};
use crate::lz4;

/// The signature of the only container format read here.
pub const SIGNATURE: &str = "UnityFS";

/// The most a block may decompress to: 64 MiB (Unity writes 128 KiB blocks).
pub const MAX_BLOCK_SIZE: u32 = 64 << 20;

/// The most a read reserves before any block is decompressed: 256 MiB. A larger range grows the
/// buffer as the blocks come out.
const MAX_RESERVE: usize = 256 << 20;

/// Bit mask of the compression method in the flags of the header and of a block.
const COMPRESSION_MASK: u32 = 0x3f;
/// Flag of a file entry that marks a serialized file (as opposed to a raw resource).
const ENTRY_SERIALIZED: u32 = 0x04;

/// How the data of a block is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// Stored as is.
    None,
    /// LZMA (not read by this crate).
    Lzma,
    /// LZ4 block format.
    Lz4,
    /// LZ4 block format, written by the slower high-compression encoder; decoded the same way.
    Lz4Hc,
    /// Some other value of the 6-bit field.
    Other(u32),
}

impl Compression {
    fn from_bits(bits: u32) -> Compression {
        match bits & COMPRESSION_MASK {
            0 => Compression::None,
            1 => Compression::Lzma,
            2 => Compression::Lz4,
            3 => Compression::Lz4Hc,
            n => Compression::Other(n),
        }
    }
}

/// The fixed part at the start of a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Format version of the container (`6` for Unity 2019.4).
    pub format_version: u32,
    /// The "player version" string, `5.x.x` in every bundle of the game.
    pub player_version: String,
    /// The Unity version that wrote the bundle, e.g. `2019.4.9f1`.
    pub unity_version: String,
    /// Size of the whole file as the header states it.
    pub size: u64,
    /// Size of the compressed block info.
    pub block_info_compressed_size: u32,
    /// Size of the block info once decompressed.
    pub block_info_uncompressed_size: u32,
    /// Flags: the low 6 bits are the compression of the block info.
    pub flags: u32,
    /// Offset of the (compressed) block info, right after the fields above.
    pub block_info_offset: u64,
}

impl Header {
    /// The compression of the block info.
    pub fn block_info_compression(&self) -> Compression {
        Compression::from_bits(self.flags)
    }
}

/// One compressed block of data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// Size once decompressed.
    pub uncompressed_size: u32,
    /// Size as stored.
    pub compressed_size: u32,
    /// Flags: the low 6 bits are the compression.
    pub flags: u16,
}

impl Block {
    /// How the block is stored.
    pub fn compression(&self) -> Compression {
        Compression::from_bits(self.flags as u32)
    }
}

/// One file of the bundle: a serialized file or a raw resource (`.resS`, `.resource`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Name of the file inside the bundle, e.g. `CAB-0123....` or `CAB-0123....resS`.
    pub path: String,
    /// Where the file starts in the concatenation of the decompressed blocks.
    pub offset: u64,
    /// Size of the file.
    pub size: u64,
    /// Flags; `4` marks a serialized file.
    pub flags: u32,
}

impl Entry {
    /// Whether the entry is a serialized file (the objects), not a raw resource.
    pub fn is_serialized(&self) -> bool {
        self.flags & ENTRY_SERIALIZED != 0
    }
}

/// A UnityFS bundle held in memory (or memory-mapped: any `AsRef<[u8]>` will do).
///
/// Opening reads and checks the header, the block table and the directory. Files are
/// decompressed on demand, and a request touches only the blocks that cover it.
#[derive(Debug)]
pub struct Bundle<T: AsRef<[u8]>> {
    data: T,
    header: Header,
    blocks: Vec<Block>,
    entries: Vec<Entry>,
    /// Offset in `data` where the first block starts
    data_start: usize,
    /// For each block: where it starts in the compressed data and in the decompressed stream
    block_starts: Vec<(u64, u64)>,
}

impl<T: AsRef<[u8]>> Bundle<T> {
    /// Opens a bundle. Fails on anything that is not a UnityFS version 6 bundle with a
    /// block info stored right after the header, or whose tables do not fit the data.
    pub fn new(data: T) -> Result<Bundle<T>> {
        let bytes = data.as_ref();
        let mut cur = Cursor::new(bytes);

        let signature = cur.cstr("signature")?;
        if signature != SIGNATURE {
            return Err(Error::Unsupported(format!(
                "bundle signature {signature:?}, only {SIGNATURE:?} is read"
            )));
        }
        let format_version = cur.u32_be("format version")?;
        if format_version != 6 {
            return Err(Error::Unsupported(format!(
                "UnityFS format version {format_version}, only 6 is read"
            )));
        }
        let player_version = cur.cstr("player version")?;
        let unity_version = cur.cstr("unity version")?;
        let size = cur.i64_be("bundle size")?;
        let block_info_compressed_size = cur.u32_be("block info size")?;
        let block_info_uncompressed_size = cur.u32_be("block info size")?;
        let flags = cur.u32_be("flags")?;
        let header = Header {
            format_version,
            player_version,
            unity_version,
            size: u64::try_from(size).map_err(|_| invalid("negative bundle size"))?,
            block_info_compressed_size,
            block_info_uncompressed_size,
            flags,
            block_info_offset: 0,
        };

        // Flags this crate knows the meaning of for version 6 bundles: the compression, and
        // bit 6 (block info and directory in one blob, which is the only layout there is).
        if flags & !(COMPRESSION_MASK | 0x40) != 0 {
            return Err(Error::Unsupported(format!(
                "bundle flags {flags:#x}: only the compression and the combined block info and directory bit are read"
            )));
        }

        let start = cur.pos();
        let (info, data_start) = read_block_info(bytes, start, &header)?;
        let mut header = header;
        header.block_info_offset = start as u64;
        let (blocks, entries) = parse_block_info(&info)?;

        let mut block_starts = Vec::with_capacity(blocks.len());
        let (mut comp, mut uncomp) = (0u64, 0u64);
        for b in &blocks {
            block_starts.push((comp, uncomp));
            comp += b.compressed_size as u64;
            uncomp += b.uncompressed_size as u64;
        }
        let available = (bytes.len() - data_start) as u64;
        if comp > available {
            return Err(Error::Truncated {
                what: "compressed blocks",
            });
        }
        // Every claim a block makes must be one its stored bytes can back, so that the sizes in
        // the table never decide how much memory is set aside.
        for (i, b) in blocks.iter().enumerate() {
            check_block(i, b)?;
        }

        Ok(Bundle {
            data,
            header,
            blocks,
            entries,
            data_start,
            block_starts,
        })
    }

    /// The header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The block table.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The files in the bundle.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// A file by its name in the bundle.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// The first serialized file that is not a `.sharedAssets` one: the file with the objects.
    pub fn main_serialized_entry(&self) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.is_serialized() && !e.path.ends_with(".sharedAssets"))
    }

    /// The bundle's bytes as given to [`Bundle::new`].
    pub fn data(&self) -> &T {
        &self.data
    }

    /// Decompresses a whole file.
    pub fn read_entry(&self, entry: &Entry) -> Result<Vec<u8>> {
        let len = usize::try_from(entry.size).map_err(|_| Error::TooLarge {
            what: "file",
            value: entry.size,
            limit: usize::MAX as u64,
        })?;
        self.read_entry_range(entry, 0, len)
    }

    /// Decompresses `len` bytes starting `start` bytes into a file. Only the blocks that cover
    /// the range are decompressed, which is what makes a few kilobytes out of a very large
    /// `.resS` cheap.
    pub fn read_entry_range(&self, entry: &Entry, start: u64, len: usize) -> Result<Vec<u8>> {
        let end = start
            .checked_add(len as u64)
            .filter(|&e| e <= entry.size)
            .ok_or_else(|| invalid("range reaches past the end of the file"))?;
        let abs_start = entry
            .offset
            .checked_add(start)
            .ok_or_else(|| invalid("file offset overflows"))?;
        let abs_end = entry
            .offset
            .checked_add(end)
            .ok_or_else(|| invalid("file offset overflows"))?;
        let total = self
            .block_starts
            .last()
            .zip(self.blocks.last())
            .map(|(s, b)| s.1 + b.uncompressed_size as u64)
            .unwrap_or(0);
        if abs_end > total {
            return Err(invalid("file lies outside the decompressed blocks"));
        }
        if len == 0 {
            return Ok(Vec::new());
        }

        // The first block whose end is past the start of the range
        let first = self
            .block_starts
            .partition_point(|&(_, u)| u <= abs_start)
            .saturating_sub(1);

        // The decompressed blocks are appended here
        let mut out: Vec<u8> = Vec::new();
        let _ = out.try_reserve_exact(len.min(MAX_RESERVE));
        let mut index = first;
        while (out.len() as u64) < len as u64 {
            let block = self
                .blocks
                .get(index)
                .ok_or_else(|| invalid("file lies outside the decompressed blocks"))?;
            let (comp_at, uncomp_at) = self.block_starts[index];
            let unpacked = self.decompress_block(index, block, comp_at)?;
            let from = abs_start.max(uncomp_at) - uncomp_at;
            let want = (len - out.len()) as u64;
            let to = (from + want).min(unpacked.len() as u64);
            let part = unpacked
                .get(from as usize..to as usize)
                .ok_or_else(|| invalid("block table does not cover the file"))?;
            out.extend_from_slice(part);
            index += 1;
        }
        Ok(out)
    }

    fn decompress_block(&self, index: usize, block: &Block, comp_at: u64) -> Result<Vec<u8>> {
        let from = self.data_start + comp_at as usize;
        let stored = &self.data.as_ref()[from..from + block.compressed_size as usize];
        decode(
            block.compression(),
            stored,
            block.uncompressed_size,
            Some(index),
        )
    }
}

fn check_block(index: usize, b: &Block) -> Result<()> {
    if b.uncompressed_size > MAX_BLOCK_SIZE {
        return Err(Error::TooLarge {
            what: "decompressed block",
            value: b.uncompressed_size as u64,
            limit: MAX_BLOCK_SIZE as u64,
        });
    }
    match b.compression() {
        Compression::None if b.compressed_size != b.uncompressed_size => Err(invalid(format!(
            "block {index} is stored but its two sizes differ"
        ))),
        Compression::None => Ok(()),
        Compression::Lz4 | Compression::Lz4Hc => {
            if lz4::plausible_size(b.compressed_size as u64, b.uncompressed_size as u64) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "block {index}: {} bytes cannot decompress to {}",
                    b.compressed_size, b.uncompressed_size
                )))
            }
        }
        other => Err(Error::Unsupported(format!(
            "block {index}: compression {other:?}"
        ))),
    }
}

/// Decompresses stored bytes that should become `uncompressed` bytes.
fn decode(
    compression: Compression,
    stored: &[u8],
    uncompressed: u32,
    index: Option<usize>,
) -> Result<Vec<u8>> {
    let what = match index {
        Some(i) => format!("block {i}"),
        None => "block info".to_string(),
    };
    if uncompressed > MAX_BLOCK_SIZE {
        return Err(Error::TooLarge {
            what: "decompressed block",
            value: uncompressed as u64,
            limit: MAX_BLOCK_SIZE as u64,
        });
    }
    match compression {
        Compression::None => {
            if stored.len() != uncompressed as usize {
                return Err(Error::Decompress(format!(
                    "{what}: stored block of {} bytes should have {uncompressed}",
                    stored.len()
                )));
            }
            Ok(stored.to_vec())
        }
        Compression::Lz4 | Compression::Lz4Hc => {
            if uncompressed == 0 && stored.is_empty() {
                return Ok(Vec::new());
            }
            if !lz4::plausible_size(stored.len() as u64, uncompressed as u64) {
                return Err(Error::Decompress(format!(
                    "{what}: {} bytes cannot decompress to {uncompressed}",
                    stored.len()
                )));
            }
            lz4::decompress(stored, uncompressed as usize)
                .map_err(|e| Error::Decompress(format!("{what}: {e}")))
        }
        other => Err(Error::Unsupported(format!("{what}: compression {other:?}"))),
    }
}

/// Decompresses the block info, which follows the header directly. Returns it and the offset
/// where the blocks start.
///
/// The rabex fork this crate was compared with accepts zero padding to a multiple of 16 before the
/// block info in 2019.4 bundles. None of the 3,088 bundles of the game has any (see
/// `docs/FORMATS.md`), so that is not read here: such a bundle fails with a decompression error.
fn read_block_info(bytes: &[u8], start: usize, header: &Header) -> Result<(Vec<u8>, usize)> {
    let comp = header.block_info_compressed_size as usize;
    let stored = start
        .checked_add(comp)
        .and_then(|e| bytes.get(start..e))
        .ok_or(Error::Truncated { what: "block info" })?;
    let info = decode(
        header.block_info_compression(),
        stored,
        header.block_info_uncompressed_size,
        None,
    )?;
    Ok((info, start + comp))
}

fn parse_block_info(info: &[u8]) -> Result<(Vec<Block>, Vec<Entry>)> {
    let mut cur = Cursor::new(info);
    let _hash = cur.array::<16>("block info hash")?;

    let block_count = cur.i32_be("block count")?;
    // A block record is 10 bytes; the count cannot be larger than what remains can hold
    let block_count = checked_count(block_count, 10, cur.remaining(), "block count")?;
    let mut blocks = Vec::with_capacity(block_count);
    for _ in 0..block_count {
        blocks.push(Block {
            uncompressed_size: cur.u32_be("block")?,
            compressed_size: cur.u32_be("block")?,
            flags: cur.u16_be("block")?,
        });
    }

    let entry_count = cur.i32_be("file count")?;
    // An entry is 21 bytes at least (offset, size, flags, a NUL)
    let entry_count = checked_count(entry_count, 21, cur.remaining(), "file count")?;
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let offset = cur.i64_be("file entry")?;
        let size = cur.i64_be("file entry")?;
        let flags = cur.u32_be("file entry")?;
        let path = cur.cstr("file name")?;
        entries.push(Entry {
            path,
            offset: u64::try_from(offset).map_err(|_| invalid("negative file offset"))?,
            size: u64::try_from(size).map_err(|_| invalid("negative file size"))?,
            flags,
        });
    }
    Ok((blocks, entries))
}

/// A count read from the input, checked to be non-negative and to fit in what remains.
pub(crate) fn checked_count(
    count: i32,
    min_record: usize,
    remaining: usize,
    what: &'static str,
) -> Result<usize> {
    let n = usize::try_from(count).map_err(|_| invalid(format!("negative {what}")))?;
    if n.saturating_mul(min_record) > remaining {
        return Err(Error::TooLarge {
            what,
            value: n as u64,
            limit: (remaining / min_record) as u64,
        });
    }
    Ok(n)
}
