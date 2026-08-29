//! The overflow-POLICY boundary types — the `(pragma overflow …)` / `Project.cdz` overflow-signed/
//! unsigned directive as pure data. Part of the compile-boundary contract `cdz` and `rcdzc` agree on:
//! `cdz` builds an `OverflowSpec` from the manifest + threads it across the delegate seam as
//! `--overflow-signed`/`--overflow-unsigned` flags, and `rcdzc` reads it into `db.global_overflow` +
//! the per-module policy. `rcdzc::db` `pub use`s these so `rcdzc::db::{OverflowMode, OverflowSpec}` and
//! every internal `crate::db::` ref stay byte-stable. Pure data (no deps): the signed/unsigned SELECTION
//! for a node stays in `rcdzc::infer` (post-monomorphization); this is only the load-time source.

/// How an unqualified fixed-width integer arithmetic operator (`+`/`-`/`*`) behaves when its result
/// overflows the operand type: `Trap` rejects/aborts (the language default — `numeric-model.md` §Overflow),
/// `Wrap` computes modulo 2^width. Selected per node by the governing `(pragma overflow …)` (or the global
/// `Project.cdz` manifest, or the `Trap` default). A named `Int64.wrapping-*` / `*.checked-*` form is IMMUNE
/// — it carries its own overflow contract and is never governed by the pragma.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverflowMode {
    /// Overflow is a fault (reject at compile time for a constant, abort at run time otherwise). The default.
    Trap,
    /// Overflow wraps modulo 2^width (two's-complement for signed).
    Wrap,
}

/// A module's declared overflow policy — the `(pragma overflow (signed <mode>) (unsigned <mode>))`
/// directive, split by operand SIGNEDNESS: `signed` governs an op over a signed type (`Int8`…`Int64`),
/// `unsigned` an op over an unsigned type (`UInt8`…`UInt64`). Either sub-form may be absent (`None`), in
/// which case that signedness falls through to the next precedence level (global manifest, then `Trap`).
/// The signed/unsigned SELECTION for a given node is deferred to `infer` (post-monomorphization, once the
/// operand's concrete signedness is known); this pair is the load-time source the selection reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct OverflowSpec {
    /// Mode for an op over a SIGNED integer type, or `None` if the pragma omits `(signed …)`.
    pub signed: Option<OverflowMode>,
    /// Mode for an op over an UNSIGNED integer type, or `None` if the pragma omits `(unsigned …)`.
    pub unsigned: Option<OverflowMode>,
}
