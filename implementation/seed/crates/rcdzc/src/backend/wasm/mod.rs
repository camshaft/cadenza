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
use crate::backend::wasm::select::{SelectedFunc, select_function};
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// Emit a WebAssembly component for the program in `db` under the boundary `layout`. Selects each
/// definition in the layout's emission order, serializes the core module, and assembles the envelope.
pub fn emit(db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    // Select each reachable definition's body, in emission order, WITH its parameters — so a
    // parameterized exported function selects to a real wasm function (params → local slots, body →
    // machine ops). `order[k]` is `exports[k]`'s def (exports first); a reachable non-export selects
    // nullary (it has no export plan — a later stage's internal callee).
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = layout
            .exports
            .iter()
            .find(|e| e.def == def)
            .map(|e| e.params.clone())
            .unwrap_or_default();
        funcs.push(select_function(db, body, &params)?);
    }

    // Serialize the embedded core module (multi-export core module, functions in emission order).
    let core = serialize::core_module(&funcs, layout).map_err(Reject::decline)?;

    // Build the component-boundary export list (each export's parameter + result valtypes) and
    // assemble the envelope. Export `k` in the layout lifts core func `k` (exports first, in order).
    let mut boundary: Vec<BoundaryExport> = Vec::new();
    for e in &layout.exports {
        let result = serialize::export_result_valtype(&e.result).map_err(Reject::decline)?;
        // Each parameter's COMPONENT-boundary valtype (distinct from the core valtype — a signed 64
        // integer is `s64` at the boundary, `i64` in the core). Reuses the result mapping per param.
        let mut params = Vec::new();
        for (_, ty) in &e.params {
            let vt = serialize::export_result_valtype(ty)
                .map_err(Reject::decline)?
                .ok_or_else(|| Reject::decline("a parameter type has no component valtype"))?;
            params.push(vt);
        }
        boundary.push(BoundaryExport {
            name: e.name.clone(),
            params,
            result,
        });
    }

    Ok(envelope::assemble(&core, &boundary))
}

/// The AST body occurrence of definition `def`, or a decline if it is malformed (no body).
fn def_body(db: &Db, def: usize) -> Result<crate::ast::StructId, Reject> {
    db.defs[def]
        .body
        .ok_or_else(|| Reject::decline(format!("definition `{}` has no body", db.defs[def].name)))
}
