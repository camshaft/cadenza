//! Base62 encoding of a 33-byte content hash — the ONE text form the CAS keys blobs by (`GET /{hash}`). A
//! handler that publishes a blob (`blobs.put`) and returns its RAW 33-byte ProgramHash (e.g. the /parse route's
//! ast-hash, or a /compile component hash) hands the driver raw bytes; to fetch that blob back the driver must
//! render them as the base62 text the CAS URL expects. This mirrors `cdz-contract`'s `Hash` base62 (copied,
//! std-only, to keep this excluded crate dep-minimal): treat the 33 bytes as one big-endian 264-bit integer,
//! repeatedly divide by 62 (remainder = least-significant digit), left-pad to the fixed 45-char width.

const BYTES: usize = 33;
const CHARS: usize = 45;
const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Render a 33-byte content hash as its canonical 45-char base62 text (the CAS key). `None` if `bytes` is not
/// exactly 33 bytes (a well-formed hash is always 33: a tag byte + a 256-bit digest).
#[must_use]
pub fn encode(bytes: &[u8]) -> Option<String> {
    let mut n: [u8; BYTES] = bytes.try_into().ok()?;
    let mut out = [0u8; CHARS];
    for slot in out.iter_mut().rev() {
        // One long-division pass: n, rem = divmod(n, 62), big-endian.
        let mut rem = 0u32;
        for b in &mut n {
            let acc = (rem << 8) | u32::from(*b);
            *b = (acc / 62) as u8;
            rem = acc % 62;
        }
        *slot = ALPHABET[rem as usize];
    }
    // 45 base62 digits fully consume a 264-bit value, so the quotient is now zero.
    Some(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_non_33_byte_input() {
        assert!(encode(b"too short").is_none());
        assert!(encode(&[0u8; 32]).is_none());
        assert!(encode(&[0u8; 34]).is_none());
    }

    #[test]
    fn encodes_33_bytes_to_45_alphabet_chars() {
        let e = encode(&[0u8; 33]).unwrap();
        assert_eq!(e.len(), 45);
        assert!(e.chars().all(|c| ALPHABET.contains(&(c as u8))));
        // All-zero → all-'0'; a distinct value → a distinct, still-45-char, alphabet-only text.
        assert_eq!(e, "0".repeat(45));
        let mut v = [0u8; 33];
        v[32] = 1; // the integer 1 → base62 "…001"
        let one = encode(&v).unwrap();
        assert_eq!(one.len(), 45);
        assert!(one.ends_with('1') && one.starts_with('0'));
        assert_ne!(one, e);
    }
}
