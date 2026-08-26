//! Unsigned LEB128 varint (`VarU64`): 7 data bits per byte, high bit = continuation, up to 10 bytes,
//! overlong encodings rejected. Used for the small, slowly-growing fields of the wire form (counts,
//! node-id references). Plus fixed big-endian scalar helpers for the exact-width fields. No dependency.
//!
//! Reads are total: a truncated or over-long varint returns `None` rather than panicking, because
//! decode operates on untrusted external bytes.

// `alloc`-sourced `Vec` so this file compiles under BOTH std (rcdzc) and `#![no_std]` (cdz-runtime,
// which `include!`s it for the shared canonical serializer). `alloc::vec::Vec` == `std::vec::Vec`.
use alloc::vec::Vec;

/// Append the unsigned LEB128 encoding of `value` to `out`.
pub fn write_u64(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// A cursor reading out of a byte slice, tracking position. Reads never panic.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }

    /// Read one raw byte, or `None` at end of input.
    pub fn byte(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// Read `n` raw bytes as a slice, or `None` if fewer than `n` remain.
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Read a fixed 8-byte big-endian `u64`.
    pub fn read_u64_be(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        Some(u64::from_be_bytes(b.try_into().ok()?))
    }

    /// Read a fixed 8-byte big-endian `i64`.
    pub fn read_i64_be(&mut self) -> Option<i64> {
        let b = self.take(8)?;
        Some(i64::from_be_bytes(b.try_into().ok()?))
    }

    /// Read a `VarU64` (unsigned LEB128). `None` on truncation or a value wider than 64 bits.
    pub fn read_varu64(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.byte()?;
            if shift >= 64 {
                return None;
            }
            let payload = (byte & 0x7f) as u64;
            if shift == 63 && payload > 1 {
                return None;
            }
            result |= payload << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
        }
    }

    /// Read a `VarU64` and narrow to `usize`. `None` if it exceeds `usize`.
    pub fn read_var_len(&mut self) -> Option<usize> {
        usize::try_from(self.read_varu64()?).ok()
    }

    /// Read a big-endian `u64` length and narrow to `usize`.
    pub fn read_be_len(&mut self) -> Option<usize> {
        usize::try_from(self.read_u64_be()?).ok()
    }
}

/// Append a fixed 8-byte big-endian `u64`.
pub fn write_u64_be(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// Append a fixed 8-byte big-endian `i64`.
pub fn write_i64_be(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varu64_round_trips() {
        for v in [0u64, 1, 127, 128, 300, 16_384, u64::MAX, u64::MAX - 1] {
            let mut buf = Vec::new();
            write_u64(&mut buf, v);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_varu64(), Some(v), "value {v}");
            assert!(r.at_end());
        }
    }

    #[test]
    fn varu64_is_compact() {
        // 0..=127 -> 1 byte, 128 -> 2 bytes, u64::MAX -> 10 bytes.
        let one = |v| {
            let mut b = Vec::new();
            write_u64(&mut b, v);
            b.len()
        };
        assert_eq!(one(0), 1);
        assert_eq!(one(127), 1);
        assert_eq!(one(128), 2);
        assert_eq!(one(u64::MAX), 10);
    }

    #[test]
    fn fixed_be_round_trips() {
        for v in [0u64, 1, 42, u64::MAX] {
            let mut buf = Vec::new();
            write_u64_be(&mut buf, v);
            assert_eq!(buf.len(), 8);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_u64_be(), Some(v));
        }
        for v in [0i64, -1, i64::MIN, i64::MAX] {
            let mut buf = Vec::new();
            write_i64_be(&mut buf, v);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_i64_be(), Some(v));
        }
    }

    #[test]
    fn truncated_is_none_not_panic() {
        assert_eq!(Reader::new(&[0x80]).read_varu64(), None);
    }

    #[test]
    fn overlong_is_none() {
        let buf = [0x80u8; 11];
        assert_eq!(Reader::new(&buf).read_varu64(), None);
    }
}
