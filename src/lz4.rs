//! LZ4 block decompression.
//!
//! The format is the one in the LZ4 block format description
//! (<https://github.com/lz4/lz4/blob/dev/doc/lz4_Block_format.md>): a sequence of
//! `token, [literal length extension], literals, offset (u16 LE), [match length extension]`,
//! the last sequence having only literals. Unity's `LZ4` and `LZ4HC` bundle compression both
//! produce this format (HC only searches harder when compressing).

use crate::error::{Error, Result};

/// The most a compressed byte can stand for: a run of 255-valued length bytes adds 255 bytes per
/// byte of input, and a match costs three bytes of input at least, so the ratio stays below this.
pub(crate) const MAX_RATIO: u64 = 255;

fn err(msg: &str) -> Error {
    Error::Decompress(format!("lz4: {msg}"))
}

/// Decompresses one block to exactly `expected` bytes.
///
/// The output buffer is allocated up front, so the caller checks `expected` against the input
/// size first (see [`plausible_size`]).
///
/// Short copies are made 16 bytes at a time, writing past the end of the copy into output that is
/// not yet written and will be overwritten (the output is zero-filled first, so this is safe code);
/// that is what makes this decoder several times faster than a byte loop.
pub(crate) fn decompress(input: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = vec![0u8; expected];
    let mut o = 0usize;
    let mut i = 0usize;
    loop {
        let token = *input.get(i).ok_or_else(|| err("missing token"))?;
        i += 1;

        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            lit = read_length(input, &mut i, lit)?;
        }
        let lit_end = i
            .checked_add(lit)
            .filter(|&e| e <= input.len())
            .ok_or_else(|| err("literals run past the input"))?;
        if lit > expected - o {
            return Err(err("output larger than the block table says"));
        }
        if lit <= 16 && i + 16 <= input.len() && o + 16 <= expected {
            // Copy 16 bytes, of which `lit` are literals; the rest is overwritten later
            out[o..o + 16].copy_from_slice(&input[i..i + 16]);
        } else {
            out[o..o + lit].copy_from_slice(&input[i..lit_end]);
        }
        o += lit;
        i = lit_end;

        if i == input.len() {
            // The last sequence has no match part
            break;
        }

        let off = input
            .get(i..i + 2)
            .ok_or_else(|| err("missing match offset"))?;
        let offset = u16::from_le_bytes([off[0], off[1]]) as usize;
        i += 2;
        if offset == 0 || offset > o {
            return Err(err("match offset outside the output"));
        }

        let mut len = (token & 15) as usize;
        if len == 15 {
            len = read_length(input, &mut i, len)?;
        }
        len += 4;
        if len > expected - o {
            return Err(err("output larger than the block table says"));
        }
        let start = o - offset;
        if offset >= 16 && o + len + 16 <= expected {
            // Copy 16 bytes at a time; the source never overlaps the 16 bytes written
            let mut k = 0;
            while k < len {
                let chunk: [u8; 16] = out[start + k..start + k + 16].try_into().unwrap();
                out[o + k..o + k + 16].copy_from_slice(&chunk);
                k += 16;
            }
        } else if offset >= len {
            out.copy_within(start..start + len, o);
        } else if offset == 1 {
            let b = out[start];
            out[o..o + len].fill(b);
        } else {
            // The match overlaps what it writes: the output repeats the last `offset` bytes.
            // Copy that period, then twice as much, and so on.
            let mut done = 0;
            let mut chunk = offset;
            while done < len {
                let n = chunk.min(len - done);
                out.copy_within(start..start + n, o + done);
                done += n;
                chunk *= 2;
            }
        }
        o += len;
    }
    if o != expected {
        return Err(err("output smaller than the block table says"));
    }
    Ok(out)
}

fn read_length(input: &[u8], i: &mut usize, mut len: usize) -> Result<usize> {
    loop {
        let b = *input.get(*i).ok_or_else(|| err("missing length byte"))?;
        *i += 1;
        len = len
            .checked_add(b as usize)
            .ok_or_else(|| err("length overflow"))?;
        if b != 255 {
            return Ok(len);
        }
    }
}

/// Whether `uncompressed` bytes can come out of `compressed` bytes at all.
pub(crate) fn plausible_size(compressed: u64, uncompressed: u64) -> bool {
    uncompressed <= compressed.saturating_mul(MAX_RATIO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_only() {
        // token 0x50: five literals, no match part because the input ends
        assert_eq!(
            decompress(&[0x50, b'h', b'e', b'l', b'l', b'o'], 5).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn match_copies_earlier_output() {
        let stream = [0x35, b'a', b'b', b'c', 0x03, 0x00, 0x10, b'x'];
        assert_eq!(decompress(&stream, 13).unwrap(), b"abcabcabcabcx");
    }

    #[test]
    fn match_may_overlap_its_own_output() {
        // one literal, then a match of 20 bytes at distance 1, then an empty last sequence
        let stream = [0x1F, b'a', 0x01, 0x00, 0x01, 0x00];
        assert_eq!(decompress(&stream, 21).unwrap(), vec![b'a'; 21]);
    }

    #[test]
    fn long_literal_run_uses_length_bytes() {
        let mut stream = vec![0xF0, 255, 45]; // 15 + 255 + 45 = 315 literals
        stream.extend(std::iter::repeat(7u8).take(315));
        assert_eq!(decompress(&stream, 315).unwrap(), vec![7u8; 315]);
    }

    #[test]
    fn empty_block() {
        assert_eq!(decompress(&[0x00], 0).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn rejects_malformed_streams() {
        // nothing at all
        assert!(decompress(&[], 0).is_err());
        // literals run past the input
        assert!(decompress(&[0x50, b'h'], 5).is_err());
        // a match at distance 0
        assert!(decompress(&[0x10, b'a', 0x00, 0x00, 0x00], 5).is_err());
        // a match that reaches before the start of the output
        assert!(decompress(&[0x10, b'a', 0x05, 0x00, 0x00], 5).is_err());
        // a match offset cut off
        assert!(decompress(&[0x10, b'a', 0x01], 5).is_err());
        // output larger than expected
        assert!(decompress(&[0x50, b'h', b'e', b'l', b'l', b'o'], 4).is_err());
        // output smaller than expected
        assert!(decompress(&[0x50, b'h', b'e', b'l', b'l', b'o'], 6).is_err());
        // a length that never ends
        assert!(decompress(&[0xF0, 255, 255, 255], 100).is_err());
    }

    #[test]
    fn length_bytes_cannot_overflow() {
        let mut stream = vec![0xF0];
        stream.extend(std::iter::repeat(255u8).take(10_000));
        assert!(decompress(&stream, 1 << 20).is_err());
    }

    #[test]
    fn the_ratio_check_bounds_the_claim() {
        assert!(plausible_size(10, 2550));
        assert!(!plausible_size(10, 2551));
        assert!(plausible_size(0, 0));
        assert!(!plausible_size(0, 1));
    }
}
