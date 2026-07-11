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
    // i32 division / remainder (signed and unsigned): 0x6D..0x70.
    pub const I32_DIV_S: u8 = 0x6D;
    pub const I32_DIV_U: u8 = 0x6E;
    pub const I32_REM_S: u8 = 0x6F;
    pub const I32_REM_U: u8 = 0x70;
    // i32 bitwise: and=0x71, or=0x72, xor=0x73.
    pub const I32_AND: u8 = 0x71;
    pub const I32_OR: u8 = 0x72;
    pub const I32_XOR: u8 = 0x73;
    // i32 shifts: shl=0x74, shr_s=0x75, shr_u=0x76.
    pub const I32_SHL: u8 = 0x74;
    pub const I32_SHR_S: u8 = 0x75;
    pub const I32_SHR_U: u8 = 0x76;
    pub const I64_ADD: u8 = 0x7C;
    pub const I64_SUB: u8 = 0x7D;
    pub const I64_MUL: u8 = 0x7E;
    // i32 comparisons (result i32 boolean): eq=0x46, ne=0x47, then signed then unsigned lt/gt/le/ge.
    pub const I32_EQ: u8 = 0x46;
    pub const I32_NE: u8 = 0x47;
    pub const I32_LT_S: u8 = 0x48;
    pub const I32_LT_U: u8 = 0x49;
    pub const I32_GT_S: u8 = 0x4A;
    pub const I32_GT_U: u8 = 0x4B;
    pub const I32_LE_S: u8 = 0x4C;
    pub const I32_LE_U: u8 = 0x4D;
    pub const I32_GE_S: u8 = 0x4E;
    pub const I32_GE_U: u8 = 0x4F;
    // i64 comparisons (result i32 boolean): eq=0x51, ne=0x52, then signed then unsigned lt/gt/le/ge.
    pub const I64_EQ: u8 = 0x51;
    pub const I64_NE: u8 = 0x52;
    pub const I64_LT_S: u8 = 0x53;
    pub const I64_LT_U: u8 = 0x54;
    pub const I64_GT_S: u8 = 0x55;
    pub const I64_GT_U: u8 = 0x56;
    pub const I64_LE_S: u8 = 0x57;
    pub const I64_LE_U: u8 = 0x58;
    pub const I64_GE_S: u8 = 0x59;
    pub const I64_GE_U: u8 = 0x5A;
    // local.set, unreachable.
    pub const LOCAL_SET: u8 = 0x21;
    pub const UNREACHABLE: u8 = 0x00;
    // i64 division / remainder (signed and unsigned): 0x7F..0x82.
    pub const I64_DIV_S: u8 = 0x7F;
    pub const I64_DIV_U: u8 = 0x80;
    pub const I64_REM_S: u8 = 0x81;
    pub const I64_REM_U: u8 = 0x82;
    // i64 bitwise: and=0x83, or=0x84, xor=0x85.
    pub const I64_AND: u8 = 0x83;
    pub const I64_OR: u8 = 0x84;
    pub const I64_XOR: u8 = 0x85;
    // i64 shifts: shl=0x86, shr_s=0x87, shr_u=0x88.
    pub const I64_SHL: u8 = 0x86;
    pub const I64_SHR_S: u8 = 0x87;
    pub const I64_SHR_U: u8 = 0x88;
}
