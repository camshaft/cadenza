//! The Cadenza backend — lower the OPTIMIZED Core back to Cadenza surface, emitting the BINARY AST.
//!
//! Unlike the wasm/rust backends (which emit a runnable artifact), this backend emits the PROGRAM
//! ITSELF back out: it walks the same target-neutral columns everything above the seam produced
//! (`core_of`/`type_of`/[`Layout`]) — i.e. the program AFTER resolution, inference, const-folding, and
//! optimization — and reconstructs a Cadenza surface AST from it, then serializes that tree with the
//! binary-AST codec ([`crate::codec::encode`]). The result is a `kind == "ast"` artifact.
//!
//! Three properties this enables (operator's mandate):
//! - **round-trip idempotence** — feeding the emitted binary AST back through `compile` yields the
//!   identical optimized program, hence a byte-identical re-emit (the correctness + optimization-
//!   inspection signal). A deterministic build order is what makes this hold: `crate::codec::encode`
//!   serializes BUILD ORDER (no canon pass), so this backend builds head-first, left-to-right, and
//!   the same Core always produces the same bytes.
//! - **syntax-system pipe** — the bytes decode straight into `cadenza-syntax` for sexpr/ML rendering,
//!   so a human can inspect what lowering + folding did.
//! - **lean-oracle input** — the oracle consumes binary AST directly.
//!
//! Like the other backends it consumes the structured Core DIRECTLY (no flat wasm `Lir`) and uses only
//! `backend::common`, never the sibling backends' internals. It DECLINES (attributed to this target) a
//! construct it does not yet reconstruct — the same decline-don't-miscompile discipline the wasm/rust
//! backends follow. This is the B0 slice: whole-program shape (`(do (def …)… (export …)…)`) with
//! CONSTANT-bodied nullary definitions; every non-constant Core node and every parameterized definition
//! declines, to be filled in by later increments (ops/control, binding, calls, data, …).

use crate::ast::{Builder, Leaf, Radix, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;
use crate::lower::core_of;

/// Emit the binary-AST artifact for the program in `db` under `layout`. Reconstructs a Cadenza surface
/// tree `(do (def …)… (export …)…)` over the same reachable definition set (`layout.order`) the other
/// backends emit, then serializes it with the binary-AST codec.
pub fn emit(db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    let mut b = Builder::new();

    // A top-level `(do …)` root: link unwraps a `do`/`module` root and contributes its children, so a
    // `(do <def>… <export>…)` re-reads to the same program `(module m <def>… <export>…)` would. Build
    // the head first (head-first order is the fleet-wide build convention the no-canon codec relies on).
    let do_head = b.name("do");
    let mut root_children = vec![do_head];

    // One `(def …)` per reachable definition, in layout order (a stable, target-neutral order).
    for &def in &layout.order {
        root_children.push(emit_def(db, &mut b, def)?);
    }

    // Then the exports, so recompiling the emitted program reaches the SAME definition set (an export-
    // less program would close to an empty `layout.order` and re-emit `(do)` — not idempotent). Emit in
    // the same layout order for determinism.
    for &def in &layout.order {
        if let Some(e) = layout.export_plan(def) {
            root_children.push(emit_export(db, &mut b, def, e)?);
        }
    }

    let root = b.list(root_children);
    let arenas = b.finish(root);
    Ok(crate::codec::encode(&arenas))
}

/// Reconstruct `(def (<name>) <body>)` for definition `def`. B0 handles only NULLARY definitions with a
/// CONSTANT body; a parameterized definition declines (parameter binding is a later increment).
fn emit_def(db: &mut Db, b: &mut Builder, def: usize) -> Result<StructId, Reject> {
    let name = db.defs[def].name.clone();
    let body = db.defs[def].body.ok_or_else(|| {
        Reject::decline(format!(
            "definition `{name}` has no body to lower to Cadenza"
        ))
    })?;

    // A parameterized definition needs its parameter names woven into the signature AND parameter
    // references (`Core::Param`/`Core::LocalRef`) rendered in the body — a later increment (B3).
    let params = crate::layout::def_params(db, def);
    if !params.is_empty() {
        return Err(Reject::decline(format!(
            "the Cadenza backend does not yet lower a definition with parameters (`{name}`) — B0 emits \
             constant-bodied nullary definitions only"
        )));
    }

    let def_head = b.name("def");
    // The signature `(<name>)` — a one-element list whose sole child is the def's source name. (A def
    // name is a plain identifier, emitted as a `Name` atom.)
    let sig_name = b.name(name.as_str());
    let sig = b.list(vec![sig_name]);
    let body_node = emit_expr(db, b, body)?;
    Ok(b.list(vec![def_head, sig, body_node]))
}

/// Reconstruct `(export <name>)` for an exported definition. B0 handles only an export whose boundary
/// name equals the definition's source name (an unrenamed export); a renamed export (`export … as …`)
/// declines, since dropping the rename would not round-trip.
fn emit_export(
    db: &mut Db,
    b: &mut Builder,
    def: usize,
    e: &crate::layout::ExportPlan,
) -> Result<StructId, Reject> {
    let source_name = db.defs[def].name.clone();
    if e.name != source_name {
        return Err(Reject::decline(format!(
            "the Cadenza backend does not yet lower a RENAMED export (`{source_name}` exported as \
             `{}`) — B0 emits unrenamed exports only",
            e.name
        )));
    }
    let export_head = b.name("export");
    let name_ref = b.name(source_name.as_str());
    Ok(b.list(vec![export_head, name_ref]))
}

/// Reconstruct a Cadenza surface expression from the optimized `Core` at `id`. B0 covers the constant
/// leaves only; every other node declines (attributed to this target), to be filled in by later
/// increments (ops/control B1, binding B2, calls B3, data B4, …).
fn emit_expr(db: &mut Db, b: &mut Builder, id: StructId) -> Result<StructId, Reject> {
    match core_of(db, id) {
        // An integer constant re-reads to the same value regardless of the base its text used (the base
        // is display-only and Core does not retain it), so emit the canonical decimal spelling.
        Core::ConstInt(v) => Ok(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        Core::ConstBool(bo) => Ok(b.atom_leaf(Leaf::Bool(bo))),
        Core::ConstStr(s) => Ok(b.atom_leaf(Leaf::Str(s))),
        Core::ConstChar(c) => Ok(b.atom_leaf(Leaf::Char(c))),
        // A finite float constant carries its exact `Decimal` (no `f64` rounding), which re-reads to the
        // same leaf. (`ConstFloatNan` has no finite `Decimal` and no plain written form — a later slice.)
        Core::ConstFloat(d) => Ok(b.atom_leaf(Leaf::Float(d))),
        other => Err(Reject::decline(format!(
            "the Cadenza backend does not yet lower this Core node back to Cadenza: {}",
            core_node_kind(&other)
        ))),
    }
}

/// A short human-readable kind name for a `Core` node, for the decline message (so a decline says WHICH
/// construct is not yet lowered rather than an opaque debug dump).
fn core_node_kind(c: &Core) -> &'static str {
    match c {
        Core::ConstInt(_) => "ConstInt",
        Core::ConstRational(..) => "ConstRational",
        Core::ConstBool(_) => "ConstBool",
        Core::ConstStr(_) => "ConstStr",
        Core::ConstChar(_) => "ConstChar",
        Core::ConstFloat(_) => "ConstFloat",
        Core::ConstFloatNan => "ConstFloatNan",
        Core::Unit => "Unit",
        _ => "a non-constant node",
    }
}

#[cfg(test)]
mod tests;
