//! WebAssembly opcode table generation — `wasm-encoder` is the authoritative source of the bytes.
//!
//! Both compiler implementations hand-encode wasm instructions and so both need the opcode bytes:
//! the Rust seed (`codegen.rs`'s `mod op`) and the Cadenza compiler (`compiler/compiler.cdz`, which
//! today hardcodes `0x42`/`0x10`/`0x20`/… inline). Those magic numbers are the WebAssembly spec's,
//! not ours — the thing that could be wrong is the *byte*, and the authoritative encoder of that byte
//! is `wasm-encoder` itself. So we do NOT maintain a numeric table: we keep a curated list of the
//! opcodes the compiler actually emits, mapped to their `wasm_encoder::Instruction`, and derive each
//! opcode byte by ENCODING the instruction and taking its leading byte(s). `wasm-encoder` tracks the
//! spec across versions; we only curate *which* ops we use and *what we name them*, and emit the same
//! table into BOTH compilers so they can never disagree on a byte.
//!
//! (Why not iterate all `Instruction` variants? The enum has no iterator, most variants need a typed
//! payload to construct, and the compiler uses ~50 of ~500 ops — a curated subset keyed by the names
//! the compiler already references is the right grain. The bytes are still authoritative.)

use std::path::Path;
use wasm_encoder::{BlockType, Encode, Ieee64, Instruction, MemArg};

/// The opcodes both compilers emit, as `(SCREAMING_SNAKE name, Instruction)`. The name is what each
/// compiler references; the Instruction is only used to DERIVE the byte (its payload is a throwaway
/// placeholder — the leading opcode byte is independent of the immediate). Grouped as in `mod op`.
/// A memarg/blocktype/index immediate contributes trailing bytes we drop: `opcode_byte` keeps only
/// the leading byte(s) that ARE the opcode (all of these are single-byte opcodes in the MVP).
fn opcodes() -> Vec<(&'static str, Instruction<'static>)> {
    let m = MemArg { offset: 0, align: 0, memory_index: 0 };
    vec![
        // Control / calls / locals.
        ("UNREACHABLE", Instruction::Unreachable),
        ("CALL", Instruction::Call(0)),
        ("DROP", Instruction::Drop),
        ("LOCAL_GET", Instruction::LocalGet(0)),
        ("LOCAL_SET", Instruction::LocalSet(0)),
        ("LOCAL_TEE", Instruction::LocalTee(0)),
        ("I32_CONST", Instruction::I32Const(0)),
        ("I64_CONST", Instruction::I64Const(0)),
        ("F64_CONST", Instruction::F64Const(Ieee64::new(0))),
        ("IF", Instruction::If(BlockType::Empty)),
        ("ELSE", Instruction::Else),
        ("END", Instruction::End),
        ("BLOCK", Instruction::Block(BlockType::Empty)),
        ("LOOP", Instruction::Loop(BlockType::Empty)),
        ("BR", Instruction::Br(0)),
        ("BR_IF", Instruction::BrIf(0)),
        ("RETURN", Instruction::Return),
        // i64 arithmetic / bitwise.
        ("I64_ADD", Instruction::I64Add),
        ("I64_SUB", Instruction::I64Sub),
        ("I64_MUL", Instruction::I64Mul),
        ("I64_DIV_S", Instruction::I64DivS),
        ("I64_REM_S", Instruction::I64RemS),
        ("I64_AND", Instruction::I64And),
        ("I64_OR", Instruction::I64Or),
        ("I64_XOR", Instruction::I64Xor),
        ("I64_SHL", Instruction::I64Shl),
        ("I64_SHR_S", Instruction::I64ShrS),
        // i64 comparison (result i32).
        ("I64_EQZ", Instruction::I64Eqz),
        ("I64_EQ", Instruction::I64Eq),
        ("I64_NE", Instruction::I64Ne),
        ("I64_LT_S", Instruction::I64LtS),
        ("I64_GT_S", Instruction::I64GtS),
        ("I64_LE_S", Instruction::I64LeS),
        ("I64_GE_S", Instruction::I64GeS),
        ("I64_GE_U", Instruction::I64GeU),
        // i32 test / comparison (Bool ordering + boolean negate/test). Bool is stored as i32 (false=0,
        // true=1), so its total order (false < true) is the UNSIGNED i32 comparison, and equality is
        // i32.eq.
        ("I32_EQZ", Instruction::I32Eqz),
        ("I32_EQ", Instruction::I32Eq),
        ("I32_LT_U", Instruction::I32LtU),
        ("I32_GT_U", Instruction::I32GtU),
        ("I32_LE_U", Instruction::I32LeU),
        ("I32_GE_U", Instruction::I32GeU),
        // i32 arithmetic (heap pointer math).
        ("I32_ADD", Instruction::I32Add),
        ("I32_SUB", Instruction::I32Sub),
        ("I32_MUL", Instruction::I32Mul),
        // Linear-memory loads / stores (memarg immediate dropped).
        ("I32_LOAD", Instruction::I32Load(m)),
        ("I64_LOAD", Instruction::I64Load(m)),
        ("F64_LOAD", Instruction::F64Load(m)),
        ("I32_LOAD8_U", Instruction::I32Load8U(m)),
        ("I32_STORE", Instruction::I32Store(m)),
        ("I64_STORE", Instruction::I64Store(m)),
        ("F64_STORE", Instruction::F64Store(m)),
        ("I32_STORE8", Instruction::I32Store8(m)),
        // Globals (the bump pointer lives in global 0).
        ("GLOBAL_GET", Instruction::GlobalGet(0)),
        ("GLOBAL_SET", Instruction::GlobalSet(0)),
    ]
}

/// The opcode byte of an instruction: encode it and take the leading byte. Every opcode in `opcodes()`
/// is a single-byte MVP opcode, so the immediate (index/memarg/blockty) is the *rest* of the encoding
/// and the opcode is byte 0. Asserts the encoding is non-empty (a wasm-encoder contract).
fn opcode_byte(i: &Instruction<'static>) -> u8 {
    let mut buf = Vec::new();
    i.encode(&mut buf);
    assert!(!buf.is_empty(), "instruction encoded to zero bytes");
    buf[0]
}

/// Render the Rust `op.rs` — the seed compiler's opcode constants, as a MODULE BODY (bare `pub
/// const`s + a crate-level `#![allow(dead_code)]`), so `codegen.rs` includes it with
/// `#[path = "op.rs"] mod op;` and references `op::CALL`. Some entries (memory loads/stores,
/// `local.tee`, `return`) are forward references for Phase-D memory work, hence dead-code-tolerant.
fn render_rust(ops: &[(&str, u8)]) -> String {
    let mut s = String::new();
    s.push_str(RUST_HEADER);
    s.push_str("\n#![allow(dead_code)]\n\n");
    for (name, byte) in ops {
        s.push_str(&format!("pub const {name}: u8 = 0x{byte:02X};\n"));
    }
    s
}

/// Render the Cadenza `op.cdz` — the same opcodes as a Cadenza record of byte constants, so the
/// Cadenza compiler stops hardcoding `0x42`/`0x10`/… inline and reads them from the shared table.
/// A record of `(NAME value)` fields: `op.I32-CONST` = `0x41`. Names are kebab-cased (Cadenza style).
fn render_cadenza(ops: &[(&str, u8)]) -> String {
    let mut s = String::new();
    s.push_str(CDZ_HEADER);
    s.push_str("\n(def op\n  (doc \"WebAssembly opcode bytes — the SAME table the Rust seed's `mod op` is\n");
    s.push_str("        generated from, so both compiler implementations agree on every byte.\")\n");
    s.push_str("  (record\n");
    for (name, byte) in ops {
        s.push_str(&format!("    ({} 0x{byte:02X})\n", kebab(name)));
    }
    s.push_str("  ))\n");
    s
}

/// SCREAMING_SNAKE → kebab-case (`I32_CONST` → `i32-const`), the Cadenza spelling.
fn kebab(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}

const RUST_HEADER: &str = "\
// @generated by `cargo run -p xtask -- build` from xtask/src/opcodes.rs. DO NOT EDIT.
//
// WebAssembly opcode bytes, derived by ENCODING each `wasm_encoder::Instruction` and taking its
// leading byte — `wasm-encoder` is the authoritative source of the spec's opcode numbers, so this
// table cannot drift from the standard. The SAME opcodes are emitted into `compiler/op.cdz` for the
// Cadenza compiler. See spec/learnings/2026-07-06-the-envelope-blobs-are-generated-from-the-runtime-contract.md.
";

const CDZ_HEADER: &str = "\
; @generated by `cargo run -p xtask -- build` from xtask/src/opcodes.rs. DO NOT EDIT.
;
; WebAssembly opcode bytes for the Cadenza-authored compiler — the SAME table the Rust seed's
; `mod op` is generated from (via wasm-encoder, the authoritative source of the spec's opcode
; numbers), so both compiler implementations agree on every byte. This is the magic-value sharing
; the code generator makes possible: one source of truth feeds both compilers.";

/// Generate `op.rs` (Rust) and `op.cdz` (Cadenza) from the opcode table. `seed` is the seed root;
/// `repo` is the repository root (for `implementation/compiler/op.cdz`). Returns whether either
/// changed (write-if-changed, so an unchanged table leaves both files — and cargo's cache — alone).
pub fn generate(seed: &Path, repo: &Path) -> Result<bool, String> {
    let table: Vec<(&str, u8)> = opcodes().iter().map(|(n, i)| (*n, opcode_byte(i))).collect();

    // Sanity: no duplicate names, no duplicate bytes-per-name mistake (a byte MAY legitimately be
    // shared across names? no — each opcode is distinct, so duplicate bytes signal a wrong variant).
    let mut seen_bytes = std::collections::HashMap::new();
    for (name, byte) in &table {
        if let Some(prev) = seen_bytes.insert(*byte, *name) {
            return Err(format!("opcode 0x{byte:02X} maps to both `{prev}` and `{name}` — a wrong variant?"));
        }
    }

    let rs = render_rust(&table);
    let cdz = render_cadenza(&table);
    let rs_path = seed.join("crates/cdz-compiler/src/op.rs");
    // The Cadenza table is a MERGE INPUT of the rewritten compiler `cdzc.cdz` (built by
    // implementation/compiler/Makefile from `cdzc/*.cdz`), so it lands under that source dir. The `05-`
    // prefix places it in the merge's sort order (a foundational data table, alongside the byte primitives).
    let cdz_path = repo.join("implementation/compiler/cdzc/05-op.cdz");
    let a = crate::write_if_changed(&rs_path, &rs).map_err(|e| format!("write {}: {e}", rs_path.display()))?;
    let b = crate::write_if_changed(&cdz_path, &cdz).map_err(|e| format!("write {}: {e}", cdz_path.display()))?;
    Ok(a || b)
}
