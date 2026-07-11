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
// The GENERATED value-heap runtime-ABI table (`cargo xtask codegen`, from the runtime WIT + the built
// runtime's content hash) — the structured op signatures + typed `OPS` accessor the per-program import
// section is built from (value-heap H1+). Not yet consumed (no runtime import emitted until compound
// values land), so its items are dead for now; `cargo xtask codegen --check` (a hard gate in
// `xtask check`) keeps it current with the runtime.
#[allow(dead_code)]
pub mod runtime_abi;
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
    // parameterized function (exported OR an internal callee reached by a runtime `Core::Call`) selects
    // to a real wasm function (params → local slots, body → machine ops). An EXPORT's params come from
    // its plan (which already solved boundary valtypes); a reachable NON-export callee (a recursive
    // function) reads its params via `layout::def_params` (core valtypes only — it never crosses the
    // boundary).
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.exports.iter().find(|e| e.def == def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function(db, body, &params, layout)?);
    }

    // The per-program runtime imports (ordered as `layout` numbered them). Empty until a `Core`
    // compound op lowers to a heap call (value-heap H2 computes the used-set); an empty set means no
    // import section and no index shift — byte-identical to a runtime-free program.
    let imports: Vec<&crate::backend::wasm::runtime_abi::RtOp> = Vec::new();

    // Serialize the embedded core module (multi-export core module, functions in emission order).
    let core = serialize::core_module(&funcs, &imports, layout).map_err(Reject::decline)?;

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

#[cfg(test)]
mod runtime_abi_tests {
    use super::runtime_abi::{CoreValType, OPS, RUNTIME_IFACE, RUNTIME_OPS};

    /// The generated ABI carries the known product/sum op signatures from the WIT — a guard that
    /// `xtask codegen` faithfully maps the WIT types to core valtypes (arr-get borrows an index → i32,
    /// sum-new pairs two handles → i32). Pins the H0 done-criterion: the structured data is correct.
    #[test]
    fn generated_ops_match_the_known_signatures() {
        // `arr-get(arr, index) -> elem` : two i32 params (handle + index) → an i32 handle.
        assert_eq!(OPS.arr_get.name, "arr-get");
        assert_eq!(OPS.arr_get.params, &[CoreValType::I32, CoreValType::I32]);
        assert_eq!(OPS.arr_get.result, Some(CoreValType::I32));
        // `sum-new(disc, payload) -> handle`.
        assert_eq!(OPS.sum_new.name, "sum-new");
        assert_eq!(OPS.sum_new.params, &[CoreValType::I32, CoreValType::I32]);
        // `box-int(s64) -> handle` : the one i64 param op.
        assert_eq!(OPS.box_int.params, &[CoreValType::I64]);
        // `dup(handle)` : a borrow op with NO result.
        assert_eq!(OPS.dup.result, None);
        assert_eq!(RUNTIME_IFACE, "cadenza:runtime/heap");
    }

    /// Every `OPS` field points at the same-named entry in `RUNTIME_OPS` — the typed accessor and the
    /// iterable list agree (no offset drift in the generated struct).
    #[test]
    fn ops_accessor_agrees_with_the_list() {
        for op in [
            OPS.arr_alloc,
            OPS.arr_set,
            OPS.arr_get,
            OPS.arr_len,
            OPS.sum_disc,
        ] {
            assert!(
                RUNTIME_OPS.iter().any(|o| std::ptr::eq(o, op)),
                "OPS.{} does not point into RUNTIME_OPS",
                op.name
            );
        }
        // A lowerable op has only core-scalar params; str-new (string) is flagged unlowerable.
        assert!(OPS.arr_get.lowerable);
        assert!(!OPS.str_new.lowerable);
    }
}
