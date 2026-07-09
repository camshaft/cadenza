//! Shared wasm byte-encoding primitives — LEB128, sections, vectors. The low-level encoding used by
//! both `serialize` (the scalar path) and `heap` (the runtime-compound path). Kept in one place so
//! the two component builders can never disagree on an encoding.

/// A wasm section: `<id> <byte-length-uleb> <contents>`.
pub fn section(id: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(contents.len() + 5);
    out.push(id);
    uleb128(contents.len() as u64, &mut out);
    out.extend_from_slice(contents);
    out
}

/// A wasm vector: `<count-uleb> <items concatenated>`.
pub fn wasm_vec(count: usize, items: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(items.len() + 5);
    uleb128(count as u64, &mut out);
    out.extend_from_slice(items);
    out
}

/// Unsigned LEB128, appended to `out`.
pub fn uleb128(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// Unsigned LEB128 as a fresh `Vec`.
pub fn uleb_bytes(n: u64) -> Vec<u8> {
    let mut v = Vec::new();
    uleb128(n, &mut v);
    v
}

/// Signed LEB128 for an i64, appended to `out`.
pub fn sleb128(mut n: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7; // arithmetic shift preserves sign
        let sign_bit_set = byte & 0x40 != 0;
        if (n == 0 && !sign_bit_set) || (n == -1 && sign_bit_set) {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}
