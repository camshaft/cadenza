//! `Ty` → native Rust type — the Rust backend's value strategy for scalars.
//!
//! A backend that uses its target's native aggregates maps each Cadenza type to a target type
//! (`backends-and-targets.md` §A Compound Value's Representation Is The Backend's Choice). For the
//! scalar value language that is a total, obvious map: a Cadenza integer of an ALIASED width is the
//! Rust integer of that exact width and signedness (`Int8` → `i8`, `UInt32` → `u32`, `Int64` → `i64`,
//! `UInt64` → `u64`), so Cadenza's checked-overflow arithmetic becomes Rust's `checked_*` on the same
//! type — no representation gap. Bool → `bool`, Unit → `()`.
//!
//! A NON-ALIASED width (`UInt7`, `UInt24`, `UInt48`, …) has no native Rust type, so it maps to `None`
//! and the caller declines — the SAME boundary the wasm backend draws (`comp_valtype_of` returns
//! `None` for a non-standard width, so it cannot cross the component boundary either): a narrow
//! non-standard width is a fine INTERNAL type but has no wire/native form, so a value of it must be
//! converted (`.wrap`) to an aliased width before it crosses out. A compound, function, or type value
//! also maps to `None` in this scalar slice (compounds arrive with the native-aggregate strategy in a
//! later increment).

use crate::ty::{IntTy, Sign, Ty, Width};

/// The native Rust type for a solved Cadenza type, or `None` if this backend has no native
/// representation for it (a non-aliased integer width, a not-yet-supported compound, or an
/// unresolved/erased type). The caller turns a `None` into a decline attributed to this target.
///
/// Returns an owned `String` because a compound type is a COMPOSED spelling (a tuple `(T0, T1)`), not a
/// fixed primitive name. A scalar's mapping is still one of a fixed set (`int_type`/`bool`/`()`).
pub fn rust_type(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Int(it) => int_type(*it).map(String::from),
        Ty::Bool => Some("bool".to_string()),
        Ty::Unit => Some("()".to_string()),
        // A tuple is Rust's native tuple: `(T0, T1, …)` — each element mapped recursively (so a nested
        // tuple / a tuple of scalars composes). A 1-tuple is written `(T,)` (Rust needs the trailing
        // comma to distinguish it from a parenthesized type). An element with no native mapping declines
        // the whole tuple. (The empty tuple `Ty::Tuple([])` is distinct from `Unit` upstream, but has no
        // element to map — render it as `()`, Rust's unit/empty-tuple type.)
        Ty::Tuple(elems) => {
            if elems.is_empty() {
                return Some("()".to_string());
            }
            let mut parts = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                parts.push(rust_type(e)?);
            }
            let trailing = if parts.len() == 1 { "," } else { "" };
            Some(format!("({}{trailing})", parts.join(", ")))
        }
        // Records, sums, functions, and type/erased values have no native mapping yet.
        _ => None,
    }
}

/// The native Rust integer type for an integer of a given signedness and (grounded) width, or `None`
/// for a width Rust has no primitive for. Rust primitives exist only for 8/16/32/64/128 bits; Cadenza
/// exposes 8/16/32/64 as its aliased boundary widths, so those four (each signed and unsigned) map,
/// and any other width (`UInt7`, `UInt24`, `UInt48`, an odd width) is `None` — no native form, decline.
/// The width is GROUNDED (a still-deferred/variable width takes the default, `Int64`), exactly as the
/// wasm backend grounds an unresolved width at selection.
fn int_type(it: IntTy) -> Option<&'static str> {
    // Only a fixed width has a native primitive; a deferred/variable width grounds to the default (64),
    // matching `IntTy::ground_width`. Read the fixed axes directly so a non-aliased FIXED width (e.g.
    // `UInt 24`) is rejected rather than silently grounded to something wider.
    let width = match it.width {
        Width::Fixed(w) => w,
        Width::Deferred | Width::Var(_) => crate::ty::DEFAULT_INT_WIDTH,
    };
    let signed = match it.sign {
        Sign::Fixed(s) => s,
        Sign::Deferred | Sign::Var(_) => true,
    };
    Some(match (signed, width) {
        (true, 8) => "i8",
        (true, 16) => "i16",
        (true, 32) => "i32",
        (true, 64) => "i64",
        (false, 8) => "u8",
        (false, 16) => "u16",
        (false, 32) => "u32",
        (false, 64) => "u64",
        // Any other width (non-aliased/odd) has no native Rust primitive — decline.
        _ => return None,
    })
}

/// The UNSIGNED Rust integer type whose bit width matches an integer type's slot — the type a constant
/// bit-pattern literal is written in before casting to the signed/target type (mirroring the wasm
/// backend's `to_i64_bits`/`to_i32_bits`, which emit the two's-complement bit pattern). Used by the
/// expression emitter to write `<bits>u64 as i64` etc., so a negative signed value and an unsigned
/// value at/above the signed max share ONE spelling. `None` for a non-aliased width (which declines
/// upstream before a constant of it is emitted).
pub fn unsigned_bits_type(it: IntTy) -> Option<&'static str> {
    let width = match it.width {
        Width::Fixed(w) => w,
        Width::Deferred | Width::Var(_) => crate::ty::DEFAULT_INT_WIDTH,
    };
    Some(match width {
        8 => "u8",
        16 => "u16",
        32 => "u32",
        64 => "u64",
        _ => return None,
    })
}
