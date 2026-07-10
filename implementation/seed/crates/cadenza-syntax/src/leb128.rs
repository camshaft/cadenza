//! Tiny LEB128 varint helpers for the hand-rolled binary codec. No dependency.
//!
//! - Unsigned values use plain LEB128.
//! - Signed values (a `Decimal`'s base-10 exponent) use zigzag then unsigned LEB128, so small
//!   negative exponents stay small.
//! Reads are total: a truncated or over-long (`>10` byte) varint returns `None` rather than
//! panicking, because decode operates on untrusted external bytes.

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

/// Append the zigzag + unsigned LEB128 encoding of a signed `value`.
pub fn write_i64(out: &mut Vec<u8>, value: i64) {
    write_u64(out, zigzag(value));
}

/// A cursor reading LEB128 varints out of a byte slice, tracking position.
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

    /// Read an unsigned LEB128 value. `None` on truncation or a value wider than 64 bits.
    pub fn read_u64(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.byte()?;
            // 10 groups of 7 bits covers 64 bits (with the last group partial).
            if shift >= 64 {
                return None;
            }
            let payload = (byte & 0x7f) as u64;
            // Reject bits that would overflow u64 in the final group.
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

    /// Read an unsigned LEB128 length and narrow to `usize`. `None` if it exceeds `usize`.
    pub fn read_len(&mut self) -> Option<usize> {
        usize::try_from(self.read_u64()?).ok()
    }

    /// Read a zigzag + LEB128 signed value.
    pub fn read_i64(&mut self) -> Option<i64> {
        Some(unzigzag(self.read_u64()?))
    }
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u64_round_trips() {
        for v in [0u64, 1, 127, 128, 300, 16_384, u64::MAX, u64::MAX - 1] {
            let mut buf = Vec::new();
            write_u64(&mut buf, v);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_u64(), Some(v), "value {v}");
            assert!(r.at_end());
        }
    }

    #[test]
    fn i64_round_trips() {
        for v in [0i64, 1, -1, 63, -64, i64::MIN, i64::MAX, -1_000_000] {
            let mut buf = Vec::new();
            write_i64(&mut buf, v);
            let mut r = Reader::new(&buf);
            assert_eq!(r.read_i64(), Some(v), "value {v}");
            assert!(r.at_end());
        }
    }

    #[test]
    fn truncated_is_none_not_panic() {
        // A continuation bit set but no following byte.
        let mut r = Reader::new(&[0x80]);
        assert_eq!(r.read_u64(), None);
    }

    #[test]
    fn overlong_is_none() {
        // 11 bytes all with continuation bit — wider than 64 bits.
        let buf = [0x80u8; 11];
        let mut r = Reader::new(&buf);
        assert_eq!(r.read_u64(), None);
    }
}
