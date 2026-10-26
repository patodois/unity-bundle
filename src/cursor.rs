//! A bounds-checked reader over a byte slice.

use crate::error::{invalid, Error, Result};

/// Reads integers and strings from a slice; every read checks the bounds.
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn bytes(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let s = &self.data[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            None => Err(Error::Truncated { what }),
        }
    }

    pub fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N]> {
        let s = self.bytes(N, what)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    pub fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.bytes(1, what)?[0])
    }

    pub fn bool_(&mut self, what: &'static str) -> Result<bool> {
        Ok(self.u8(what)? != 0)
    }

    pub fn i8_(&mut self, what: &'static str) -> Result<i8> {
        Ok(self.u8(what)? as i8)
    }

    pub fn f32_le(&mut self, what: &'static str) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array(what)?))
    }

    pub fn f64_le(&mut self, what: &'static str) -> Result<f64> {
        Ok(f64::from_le_bytes(self.array(what)?))
    }

    pub fn u16_le(&mut self, what: &'static str) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array(what)?))
    }

    pub fn i16_le(&mut self, what: &'static str) -> Result<i16> {
        Ok(i16::from_le_bytes(self.array(what)?))
    }

    pub fn u32_le(&mut self, what: &'static str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    pub fn i32_le(&mut self, what: &'static str) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array(what)?))
    }

    pub fn u64_le(&mut self, what: &'static str) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array(what)?))
    }

    pub fn i64_le(&mut self, what: &'static str) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array(what)?))
    }

    pub fn u16_be(&mut self, what: &'static str) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array(what)?))
    }

    pub fn u32_be(&mut self, what: &'static str) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array(what)?))
    }

    pub fn i32_be(&mut self, what: &'static str) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array(what)?))
    }

    pub fn i64_be(&mut self, what: &'static str) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array(what)?))
    }

    /// Skips to the next multiple of `n` (a power of two) of the position. Padding that runs past
    /// the end is not an error by itself: the next read fails instead, so that a structure whose
    /// last field is followed by padding the writer did not count still reads.
    pub fn align(&mut self, n: usize) {
        let rem = self.pos & (n - 1);
        if rem != 0 {
            self.pos = self.pos.saturating_add(n - rem);
        }
    }

    /// A NUL-terminated UTF-8 string.
    pub fn cstr(&mut self, what: &'static str) -> Result<String> {
        let rest = &self.data[self.pos.min(self.data.len())..];
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(Error::Truncated { what })?;
        let s = std::str::from_utf8(&rest[..len])
            .map_err(|_| invalid(format!("{what} is not valid UTF-8")))?
            .to_string();
        self.pos += len + 1;
        Ok(s)
    }
}
