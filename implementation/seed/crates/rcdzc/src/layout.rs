//! `layout` — the query that computes the program's boundary surface, target-neutrally.
//!
//! Above the backend seam sits the computation of *what the program presents at its boundary*: which
//! definitions are exported, under what (verbatim) names, with what solved parameter and result
//! types, and which definitions are reachable — computed ONCE and consumed by whichever backend runs
//! (`backends-and-targets.md` §The Boundary Layout Is Computed Once, Target-Neutrally, And Reused).
//!
//! The interface is read from each export's DECLARED signature, never inferred from a body
//! (`reference-compiler.md` §The Exported Interface Is The Declared Signature). Stage 0 exports are
//! nullary, so an export's signature is its solved result type — obtained by demanding `type_of` on
//! the export's definition body (a lazy read of the type column). The export NAME crosses verbatim;
//! no export is recognized by name or body shape.
//!
//! Reachability lives here: an export drives which definitions are reached, and only reachable
//! definitions are emitted (dead code dropped once, target-neutrally). Stage 0's slice has no calls,
//! so the reachable set is exactly the exported definitions — but this is the place that set, the
//! emission order, and each function's absolute index are fixed for every backend.

use crate::ast::StructId;
use crate::db::Db;
use crate::diag::Reject;
use crate::infer::type_of;
use crate::ty::Ty;

/// One exported entry, resolved to a target-neutral boundary plan.
#[derive(Clone, PartialEq, Debug)]
pub struct ExportPlan {
    /// The name the entry crosses the boundary under — verbatim from the source.
    pub name: String,
    /// The definition (index into `db.defs`) this export names.
    pub def: usize,
    /// The AST occurrence of the definition body — the root the backend walks the core from.
    pub body: StructId,
    /// The solved parameter types, in order. Stage 0 exports are nullary (empty).
    pub params: Vec<Ty>,
    /// The solved result type the entry returns.
    pub result: Ty,
}

/// The whole boundary layout: the exported entries (declaration order), the definitions reachable
/// from them in emission order, and each reachable definition's absolute wasm-function index.
#[derive(Clone, PartialEq, Debug)]
pub struct Layout {
    pub exports: Vec<ExportPlan>,
    /// Reachable definition indices in emission order (exported first, declaration order; then the
    /// rest). Stage 0: the exported definitions. A body emitted at position `k` is wasm func `k`
    /// (no runtime import yet, so the base is 0).
    pub order: Vec<usize>,
}

impl Layout {
    /// The absolute wasm-function index of definition `def`, or `None` if it is not emitted. Stage 0
    /// base is 0 (no runtime import), so it is the definition's position in `order`.
    pub fn abs(&self, def: usize) -> Option<u32> {
        self.order.iter().position(|&d| d == def).map(|k| k as u32)
    }
}

/// Compute the boundary layout for the program in `db` — a query the "compile" request drives. Demands
/// each export's result type (a lazy `type_of`); touches only the exported/reachable definitions.
/// A program with no export is rejected: nothing is public, so there is nothing to emit.
pub fn compute(db: &mut Db) -> Result<Layout, Reject> {
    if db.exports.is_empty() {
        return Err(Reject::decline("no `(export …)`: nothing is public"));
    }

    // Resolve each export to a plan by its DECLARED signature. An export naming no definition, or a
    // parameterized definition (Stage 0 realizes only nullary), declines.
    let mut exports: Vec<ExportPlan> = Vec::new();
    for i in 0..db.exports.len() {
        let name = db.exports[i].name.clone();
        let def = match db.exports[i].def {
            Some(d) => d,
            None => {
                return Err(Reject::decline(format!(
                    "export `{name}` names no definition"
                )));
            }
        };
        if !db.defs[def].params.is_empty() {
            return Err(Reject::decline(format!(
                "export `{name}`: parameterized exports not yet supported (nullary only)"
            )));
        }
        let body = match db.defs[def].body {
            Some(b) => b,
            None => {
                return Err(Reject::decline(format!(
                    "export `{name}`: definition has no body"
                )));
            }
        };
        // The result type is the entry body's solved type — a lazy read of the type column.
        let result = type_of(db, body);
        exports.push(ExportPlan {
            name,
            def,
            body,
            params: Vec::new(),
            result,
        });
    }

    // Emission order: exported definitions first, in declaration order, deduplicated. Stage 0 has no
    // calls, so nothing further is reachable.
    let mut order: Vec<usize> = Vec::new();
    for e in &exports {
        if !order.contains(&e.def) {
            order.push(e.def);
        }
    }

    Ok(Layout { exports, order })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::scalar_program;

    #[test]
    fn one_export_by_signature() {
        let (ast, _) = scalar_program();
        let mut db = Db::load(ast);
        let layout = compute(&mut db).expect("layout");
        assert_eq!(layout.exports.len(), 1);
        assert_eq!(layout.exports[0].name, "main");
        assert!(layout.exports[0].params.is_empty());
        assert!(layout.exports[0].result.agrees_with(&Ty::int64()));
        // The single exported definition is wasm func 0.
        assert_eq!(layout.order, vec![0]);
        assert_eq!(layout.abs(0), Some(0));
    }

    #[test]
    fn no_export_declines() {
        // A program with a def but no export presents nothing — layout declines.
        use crate::ast::{Builder, IntValue, Leaf, Radix};
        let mut b = Builder::new();
        let module = b.name("module");
        let m = b.name("m");
        let def = b.name("def");
        let sig = {
            let main = b.name("main");
            b.list(vec![main])
        };
        let body = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
            radix: Radix::Dec,
        });
        let def_form = b.list(vec![def, sig, body]);
        let root = b.list(vec![module, m, def_form]);
        let mut db = Db::load(b.finish(root));
        assert!(compute(&mut db).is_err());
    }
}
