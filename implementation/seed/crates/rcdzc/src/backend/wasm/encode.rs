//! Byte-encoding primitives for the wasm backend — LEB128, sections, vectors, and the opcode table.
//!
//! These are the low-level bytes the core-module and component serializers lay. They live in the wasm
//! backend, not in a shared layer, because a raw encoding byte is a TARGET concern: another backend
//! has its own (`backends-and-targets.md` §The Flat Instruction Rung Is A Property Of A Linearizing
//! Backend; `reference-compiler.md` §The Encoding Belongs To The Serializer Alone). Hand-written in
//! plain byte pushes so the byte path ports 1:1 to the Cadenza self-host — no external encoder in the
//! compile path (the `wasm-encoder` oracle is tests-only).

/// Append the unsigned LEB128 encoding of `n` to `out`.
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

/// The unsigned LEB128 of `n` as a fresh `Vec`.
pub fn uleb_bytes(n: u64) -> Vec<u8> {
    let mut v = Vec::new();
    uleb128(n, &mut v);
    v
}

/// Append the SIGNED LEB128 encoding of an `i64` to `out`. A signed constant MUST go through this,
/// never a raw byte, so a value whose low byte has its high bit set is not sign-extended to a
/// different value (`reference-compiler.md` §A signed constant MUST be emitted through the signed
/// variable-length encoding).
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

/// The core-wasm opcode bytes this backend emits. The authoritative source is the wasm spec; the
/// tests pin these against the `wasm-encoder` oracle. Stage 0's slice needs only these.
pub mod op {
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;
    pub const IF: u8 = 0x04;
    pub const ELSE: u8 = 0x05;
    pub const END: u8 = 0x0B;
    pub const LOCAL_GET: u8 = 0x20;
    pub const I32_ADD: u8 = 0x6A;
    pub const I32_SUB: u8 = 0x6B;
    pub const I32_MUL: u8 = 0x6C;
    pub const I64_ADD: u8 = 0x7C;
    pub const I64_SUB: u8 = 0x7D;
    pub const I64_MUL: u8 = 0x7E;
}
