//! Bounds-checked little/big-endian byte cursor.

use crate::{Error, Result};

/// A cursor over a byte slice with Unity's alignment rules.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    big_endian: bool,
}

macro_rules! read_num {
    ($name:ident, $t:ty) => {
        #[doc = concat!("Reads a `", stringify!($t), "` in the cursor's byte order.")]
        #[inline]
        pub fn $name(&mut self) -> Result<$t> {
            const N: usize = std::mem::size_of::<$t>();
            let b: [u8; N] = self.take(N)?.try_into().unwrap();
            Ok(if self.big_endian {
                <$t>::from_be_bytes(b)
            } else {
                <$t>::from_le_bytes(b)
            })
        }
    };
}

impl<'a> Reader<'a> {
    /// A little-endian cursor at offset 0.
    pub fn new(data: &'a [u8]) -> Self {
        Reader {
            data,
            pos: 0,
            big_endian: false,
        }
    }

    /// Switches the byte order for subsequent reads.
    pub fn set_big_endian(&mut self, big: bool) {
        self.big_endian = big;
    }

    /// Current offset from the start of the slice.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Moves the cursor to an absolute offset.
    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(Error::Eof {
                pos,
                len: self.data.len(),
            });
        }
        self.pos = pos;
        Ok(())
    }

    /// Bytes left after the cursor.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Advances to the next multiple of 4 (Unity's `kAlignBytes`).
    #[inline]
    pub fn align4(&mut self) {
        self.pos = (self.pos + 3) & !3;
        // Aligning past the end is harmless; the next read reports EOF.
    }

    /// Returns the next `n` bytes and advances past them.
    #[inline]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let s = &self.data[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            None => Err(Error::Eof {
                pos: self.pos + n,
                len: self.data.len(),
            }),
        }
    }

    read_num!(u16, u16);
    read_num!(i16, i16);
    read_num!(u32, u32);
    read_num!(i32, i32);
    read_num!(u64, u64);
    read_num!(i64, i64);
    read_num!(f32, f32);
    read_num!(f64, f64);

    /// Reads one byte.
    #[inline]
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Reads a non-negative `i32` length prefix, checked against the bytes left
    /// (each element takes at least `min_elem` bytes).
    pub fn len_prefix(&mut self, min_elem: usize) -> Result<usize> {
        let n = self.i32()?;
        if n < 0 || (n as usize).saturating_mul(min_elem) > self.remaining() {
            return Err(Error::Format(format!(
                "bad array length {n} at {}",
                self.pos - 4
            )));
        }
        Ok(n as usize)
    }

    /// Reads a NUL-terminated UTF-8 string.
    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let n = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::Format("unterminated string".into()))?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.pos += n + 1;
        Ok(s)
    }

    /// Reads an `i32`-length-prefixed string followed by 4-byte alignment.
    pub fn aligned_string(&mut self) -> Result<String> {
        let n = self.len_prefix(1)?;
        let s = String::from_utf8_lossy(self.take(n)?).into_owned();
        self.align4();
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_alignment() {
        let data = [1u8, 0, 0, 0, 0xff, 2, 0, 0, 0, b'h', b'i', 0, 0x12, 0x34];
        let mut r = Reader::new(&data);
        assert_eq!(r.i32().unwrap(), 1);
        assert_eq!(r.u8().unwrap(), 0xff);
        r.align4();
        assert_eq!(r.pos(), 8);
        r.seek(5).unwrap();
        assert_eq!(r.u32().unwrap(), 2);
        r.seek(12).unwrap();
        r.set_big_endian(true);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert!(r.u8().is_err());
    }

    #[test]
    fn strings() {
        let data = [2u8, 0, 0, 0, b'o', b'k', 0, 0, b'a', 0];
        let mut r = Reader::new(&data);
        assert_eq!(r.aligned_string().unwrap(), "ok");
        assert_eq!(r.pos(), 8);
        assert_eq!(r.cstr().unwrap(), "a");
        let bad = [0xffu8, 0xff, 0xff, 0x7f];
        assert!(Reader::new(&bad).aligned_string().is_err());
    }
}
