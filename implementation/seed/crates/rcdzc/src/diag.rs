//! The diagnostic-code taxonomy — the ONE place every machine-readable `CDZ####` code lives.
//!
//! A rejection carries a `Code` (this enum), not a scattered string literal, so the set of codes the
//! compiler can emit is a single closed list and each code's `CDZ####` string is defined once. A new
//! diagnostic is a new variant here plus its arm in [`Code::code`]; nothing else spells a `CDZ` number.
//! The human-readable message stays free-text at the rejection site (it describes the specific
//! offending construct); the `Code` is the stable, machine-readable classification
//! (constitution §XI; options/diagnostics-schema/).

/// A machine-readable diagnostic code. Each variant is one `CDZ####` code; [`Code::code`] maps it to
/// its stable string. Grouped by family: 01xx binding/scope, 02xx type/structure, 03xx numeric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// CDZ0101 — an unbound name, or an export naming an item that is not defined.
    UnboundName,
    /// CDZ0201 — a type error: a type mismatch, a malformed/duplicate structural member (a duplicate
    /// top-level definition, a duplicate record field, a malformed record entry), a projection of an
    /// absent field or off a non-record/non-tuple, an out-of-range positional index.
    TypeError,
    /// CDZ0203 — an annotation contradiction: `(: e T)` where `e`'s inferred type does not unify with
    /// the type `T` represents (e.g. `(: 42 Bool)`, `(: (tuple 1 2) Int64)`). Distinct from CDZ0201
    /// (a structural mismatch) — this is an explicit user annotation that contradicts inference.
    AnnotMismatch,
    /// CDZ0210 — a non-exhaustive match: the arms do not cover every value of the scrutinee's type
    /// (a missing sum variant — including a nested one — a missing bool value, or an unbounded scalar
    /// with no wildcard). Type-driven, checked at compile time (core-semantics.md §Matching Is
    /// Exhaustive Or Rejected).
    NonExhaustive,
    /// CDZ0304 — a constant integer operation that has no value: it overflows, divides by zero, takes
    /// `Int64.min / -1`, or shifts out of range, detected during compile-time folding. A
    /// compile-time-knowable trap is a compile-time rejection, never a shipped runtime trap.
    ConstTrap,
    /// CDZ0305 — a compile-time-only value (a type-value, or a compound containing one) reached the
    /// runtime boundary: it survived fold and would cross into runtime code. The erasure fence — a
    /// type-value must never become a wasm value. Checked post-fold by `is_comptime_only` over every
    /// surviving Mir node.
    ComptimeErasure,
}

impl Code {
    /// The stable `CDZ####` string for this code — the single definition of each code's spelling.
    pub fn code(self) -> &'static str {
        match self {
            Code::UnboundName => "CDZ0101",
            Code::TypeError => "CDZ0201",
            Code::AnnotMismatch => "CDZ0203",
            Code::NonExhaustive => "CDZ0210",
            Code::ConstTrap => "CDZ0304",
            Code::ComptimeErasure => "CDZ0305",
        }
    }
}
