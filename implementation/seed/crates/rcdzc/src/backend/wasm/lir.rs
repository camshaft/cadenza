//! The flat instruction rung (Lir) — this backend's representation of the core.
//!
//! Lir is NOT a shared pipeline rung: it is what the wasm backend produces because its target is a
//! linear instruction stream (`backends-and-targets.md` §The Flat Instruction Rung Is A Property Of A
//! Linearizing Backend). A backend for a structured target would consume the core directly and never
//! build this. The compiler works in `ValType`/`BlockType` here; the single raw-byte encoding lives
//! in the serializer (`ValType::byte`/`BlockType::byte`), so no other pass hard-codes a wasm byte.
//!
//! This is also where the TARGET valtype for a solved [`crate::ty::Ty`] is decided: mapping a
//! language type to a wasm value type is a wasm concern, so it lives here (via [`valtype_of`]), not on
//! `Ty` (`backends-and-targets.md` §a target-specific concern lives in the target that has it).

use crate::ty::{IntTy, Ty};

/// A core-wasm value type — the machine representation a scalar takes inside a function body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValType {
    I64,
    I32,
}

impl ValType {
    /// The core-wasm valtype byte (`0x7E` i64, `0x7F` i32). The raw encoding lives here (serializer).
    pub fn byte(self) -> u8 {
        match self {
            ValType::I64 => 0x7E,
            ValType::I32 => 0x7F,
        }
    }
}

/// The type of a wasm structured block (`if`/`block`/`loop`): it leaves no value (`Empty`) or leaves
/// one value of a given type (`Val`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockType {
    Empty,
    Val(ValType),
}

impl BlockType {
    /// The block-type byte: `0x40` for empty, else the value type's byte.
    pub fn byte(self) -> u8 {
        match self {
            BlockType::Empty => 0x40,
            BlockType::Val(vt) => vt.byte(),
        }
    }
}

/// One flat wasm instruction. Frozen for the Stage-0 slice.
#[derive(Clone, PartialEq, Debug)]
pub enum Lir {
    /// `i64.const N` — a signed 64-bit constant (emitted via SLEB128).
    ConstI64(i64),
    /// `i32.const N` — a signed 32-bit constant (emitted via SLEB128).
    ConstI32(i32),
    /// `local.get I` — read local `I` (Stage 0 uses none, but the rung carries it).
    LocalGet(u32),
    /// `if <blocktype>` — a two-way branch leaving a value of the block type.
    If(BlockType),
    /// `else`.
    Else,
    /// `end`.
    End,
}

/// The wasm value type a value of solved type `ty` occupies inside a function body, or `None` for a
/// type that occupies no runtime slot (unit). An integer's width chooses i32 vs i64 (Stage 0 grounds
/// every integer to i64); a boolean is an i32. This is the wasm backend's read-off of the solved type
/// (`reference-compiler.md` §A Value's Machine Representation Follows Its Solved Type At Selection).
pub fn valtype_of(ty: &Ty) -> Option<ValType> {
    match ty {
        Ty::Int(it) => Some(int_valtype(*it)),
        Ty::Bool => Some(ValType::I32),
        Ty::Unit => None,
        // A poison's `Any` type never reaches a real machine slot (a poison fails the build before
        // emission); treat it as no representation rather than guess one.
        Ty::Any => None,
    }
}

/// The wasm value type for an integer of a given (possibly-deferred) width — its GROUND width chooses
/// the slot: ≤32 bits is an i32, otherwise an i64. A deferred width grounds to the default (64 → i64).
fn int_valtype(it: IntTy) -> ValType {
    if it.ground_width() <= 32 {
        ValType::I32
    } else {
        ValType::I64
    }
}

/// The component-model boundary valtype byte a value of solved type `ty` lowers to when it crosses the
/// component edge, or `None` for unit (a no-result export). Fixed by the component ABI
/// (`contracts/component-abi.md`): a signed integer → `s64`/`s32`, an unsigned → `u64`/`u32`, a bool →
/// `bool`. This is the wasm/component backend's boundary mapping — a target concern, kept here.
pub fn comp_valtype_of(ty: &Ty) -> Option<u8> {
    match ty {
        Ty::Int(it) => {
            let w = it.ground_width();
            // Component-model primitive valtype bytes (spec order): s32=0x7A, u32=0x79, s64=0x78,
            // u64=0x77. Stage 0 only ever emits s64 (`Int64`); the others are pinned for the widths
            // stage. (The `s64=0x78` byte matches the old compiler's oracle-checked frame.)
            Some(match (it.signed, w <= 32) {
                (true, false) => 0x78,  // s64
                (true, true) => 0x7A,   // s32
                (false, false) => 0x77, // u64
                (false, true) => 0x79,  // u32
            })
        }
        Ty::Bool => Some(0x7F), // bool
        Ty::Unit => None,
        Ty::Any => None,
    }
}
