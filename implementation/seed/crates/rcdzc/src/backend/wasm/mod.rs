//! The wasm backend — a linearizing backend that emits a WebAssembly component.
//!
//! It is a function of the typed core and the target-neutral [`Layout`]
//! (`backends-and-targets.md` §A Backend Is A Function Of The Typed Core And A Target-Neutral
//! Layout): [`emit`] selects each reachable definition's body to flat Lir (its own representation),
//! serializes them into an embedded core module, and wraps that in the N-export component envelope.
//! Every step reads columns from the `Db` on demand — the backend is the producer of the artifact
//! column, filling it by reading the earlier ones (`query-engine.md` §Producing An Artifact Is A
//! Column A Backend Fills).
//!
//! Multi-export: every `(export …)` in the layout is emitted, each by its signature ABI, under its
//! verbatim name — no single hard-coded entry.

pub mod encode;
pub mod envelope;
pub mod lir;
pub mod select;
pub mod serialize;

use crate::backend::wasm::envelope::BoundaryExport;
use crate::backend::wasm::select::{select_body, SelectedFunc};
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// Emit a WebAssembly component for the program in `db` under the boundary `layout`. Selects each
/// definition in the layout's emission order, serializes the core module, and assembles the envelope.
pub fn emit(db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    // Select each reachable definition's body, in emission order.
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        funcs.push(select_body(db, body)?);
    }

    // Serialize the embedded core module (multi-export core module, functions in emission order).
    let core = serialize::core_module(&funcs, layout).map_err(Reject::decline)?;

    // Build the component-boundary export list (each export's result valtype) and assemble the
    // envelope. Export `k` in the layout lifts core func `k` (the layout put exports first, in order).
    let mut boundary: Vec<BoundaryExport> = Vec::new();
    for e in &layout.exports {
        let result = serialize::export_result_valtype(&e.result).map_err(Reject::decline)?;
        boundary.push(BoundaryExport { name: e.name.clone(), result });
    }

    Ok(envelope::assemble(&core, &boundary))
}

/// The AST body occurrence of definition `def`, or a decline if it is malformed (no body).
fn def_body(db: &Db, def: usize) -> Result<crate::ast::StructId, Reject> {
    db.defs[def]
        .body
        .ok_or_else(|| Reject::decline(format!("definition `{}` has no body", db.defs[def].name)))
}
