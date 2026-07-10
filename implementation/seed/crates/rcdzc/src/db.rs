//! The database — the compiler's whole state as columns over one node identity, filled lazily.
//!
//! This is the query engine (`query-engine.md`). There is ONE [`Db`]; it holds the decoded AST and a
//! column per kind of fact — the resolved form, the solved type, the core form — each keyed by the
//! AST `StructId` that is the node's identity throughout. A fact is answered by a **backward** demand:
//! `type_of(id)` reads the type column; on a miss it computes the answer, which reads the resolved
//! column (filling it on demand from the AST if absent), and memoizes the result back into the type
//! column. Nothing is computed module-wide; asking one node's type touches only the nodes that answer
//! reaches. There is no cache separate from the columns and none to invalidate — incrementality is
//! re-running a query, not maintaining a graph (`query-engine.md` §Incrementality Is Re-Run, Not
//! Invalidation).
//!
//! The producers are `&mut self` methods that fill-then-return: each finishes before the next begins,
//! so the memoizing write is a plain column fill with no interior-mutability juggling — the shape
//! that ports cleanly to a Cadenza `Db` with a mutable column store.
//!
//! Absence is not a value: a column slot is either filled or not, and a reader that requires a value
//! and finds absence declines rather than defaults (`query-engine.md` §A Reader That Requires A Value
//! And Finds Absence Declines). Every negative *decision* (a decline, a reject, a poison) is itself a
//! filled value — a `Resolved::Poison` / `Core::Poison` — so it is distinguished from "not yet
//! determined".

use crate::arena::{Column, Slot};
use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::core::Core;
use crate::diag::{Code, Reject};
use crate::resolved::Resolved;
use crate::ty::Ty;

/// A top-level definition located by the one cheap top-level scan: its name, its parameter
/// occurrences (empty = nullary), and its body occurrence (absent = malformed). The body is LOCATED,
/// never entered by the scan — entering it is a later per-node demand.
#[derive(Clone, PartialEq, Debug)]
pub struct Def {
    pub name: String,
    /// Signature occurrence, for diagnostics.
    pub sig_occ: StructId,
    /// Parameter occurrences (Stage 0 realizes only nullary defs).
    pub params: Vec<StructId>,
    /// Body expression occurrence, or `None` if the def is malformed.
    pub body: Option<StructId>,
}

/// A requested export: the name to emit (verbatim) and the definition it names, resolved against the
/// scan index by name.
#[derive(Clone, PartialEq, Debug)]
pub struct Export {
    pub name: String,
    /// Index into [`Db::defs`] of the definition this export names, or `None` if no such def exists.
    pub def: Option<usize>,
}

/// The compiler's whole state for one program: the AST, the top-level index, and the lazily-filled
/// columns. Constructed once per subject program; every query is a method on it.
pub struct Db {
    /// The decoded AST — the source-of-truth column the others are derived from.
    pub ast: Arenas,
    /// The top-level definitions, from the one top-level scan (bounded; does not enter bodies).
    pub defs: Vec<Def>,
    /// The requested exports, from the scan.
    pub exports: Vec<Export>,

    /// The resolved-form column: for a node's `StructId`, what it denotes. Filled by `resolved_of`.
    resolved: Column<StructId, Resolved>,
    /// The solved-type column: for a node's `StructId`, its type. Filled by `type_of`.
    types: Column<StructId, Ty>,
    /// The core-form column: for a node's `StructId`, its A-normal form. Filled by `core_of`.
    core: Column<StructId, Core>,
}

impl Db {
    /// Build a database over a decoded program: run the one cheap top-level scan (defs + exports),
    /// leave every derived column empty. Nothing below the top level is touched until a query demands
    /// it.
    pub fn load(ast: Arenas) -> Db {
        let (defs, exports) = scan_top_level(&ast);
        Db {
            ast,
            defs,
            exports,
            resolved: Column::new(),
            types: Column::new(),
            core: Column::new(),
        }
    }

    /// The definition of the given name, if one exists — how an export resolves its target and how a
    /// later call resolves its callee (by name against the index, reading a signature, not a body).
    pub fn def_by_name(&self, name: &str) -> Option<usize> {
        let mut i = 0;
        while i < self.defs.len() {
            if self.defs[i].name == name {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    // ── The resolved column ──────────────────────────────────────────────────────────────────

    /// The resolved form of the node at `id`, filling the column on demand. Per-node: it classifies
    /// the AST occurrence and records what it denotes, leaving children as ids for a later demand.
    pub fn resolved_of(&mut self, id: StructId) -> Resolved {
        if let Slot::Filled(r) = self.resolved.get(id) {
            return r.clone();
        }
        let r = self.compute_resolved(id);
        self.resolved.fill(id, r.clone());
        r
    }

    /// Classify one AST occurrence into its resolved form. Does not recurse into children (they
    /// resolve on their own demand); a "no" is produced as a `Poison` value.
    fn compute_resolved(&self, id: StructId) -> Resolved {
        match self.ast.get(id) {
            Struct::Atom(leaf_id) => match self.ast.leaf(*leaf_id).clone() {
                // The literal's EXACT value flows through; its machine width is decided at select.
                Leaf::Int { value, .. } => Resolved::Int(value),
                Leaf::Bool(b) => Resolved::Bool(b),
                Leaf::Name(n) => {
                    // Stage 0 has no user value bindings in expression position, and unit is a form.
                    Resolved::Poison(Reject::decline(format!("unbound name `{n}`")))
                }
                Leaf::Str(_) => {
                    Resolved::Poison(Reject::decline("string literals not yet supported"))
                }
                Leaf::Float(_) => {
                    Resolved::Poison(Reject::decline("float literals not yet supported"))
                }
            },
            Struct::List(children) => {
                // `()` — the empty list — is unit.
                if children.is_empty() {
                    return Resolved::Unit;
                }
                // `(if COND THEN ELSE)`.
                if let Some(tail) = self.ast.as_form(id, "if") {
                    if tail.len() != 3 {
                        return Resolved::Poison(Reject::coded(
                            Code::Malformed,
                            "if takes exactly 3 operands",
                        ));
                    }
                    return Resolved::If { cond: tail[0], then_: tail[1], else_: tail[2] };
                }
                let head = self.ast.head_name(id).unwrap_or("<non-name head>").to_string();
                Resolved::Poison(Reject::decline(format!("unsupported form `{head}`")))
            }
        }
    }

    // ── The type column ──────────────────────────────────────────────────────────────────────

    /// The solved type of the node at `id`, filling the column on demand. Works backward: reads the
    /// resolved column (filling it if absent) and, for a compound node, its children's types — each
    /// itself a lazy `type_of`. This is the query the `type-of` request answers; asking it for one
    /// node solves only the nodes that answer reaches.
    pub fn type_of(&mut self, id: StructId) -> Ty {
        if let Slot::Filled(t) = self.types.get(id) {
            return t.clone();
        }
        let t = self.compute_type(id);
        self.types.fill(id, t.clone());
        t
    }

    /// Solve one node's type by reading its resolved form and its children's types. A poison is typed
    /// `Any` (compatible with everything) so a "no" never induces a spurious mismatch upward — the
    /// poison itself is the reported fault. An integer literal is typed with a DEFERRED width, which
    /// inference (or, failing that, the backend) grounds later.
    fn compute_type(&mut self, id: StructId) -> Ty {
        match self.resolved_of(id) {
            // A bare integer literal is polymorphic in its width until something fixes it.
            Resolved::Int(_) => Ty::int(),
            Resolved::Bool(_) => Ty::Bool,
            Resolved::Unit => Ty::Unit,
            Resolved::If { cond, then_, else_ } => {
                // Reading the children's types is the backward demand: each is a lazy `type_of`.
                let _cond_ty = self.type_of(cond);
                let then_ty = self.type_of(then_);
                let else_ty = self.type_of(else_);
                // The if's type is the join of its branches — `Any` yields the other, and a branch
                // that fixed a deferred integer width contributes it. The cond-is-Bool and
                // branches-agree CHECKS are the type-error query's job; this fills the value column.
                then_ty.join(&else_ty)
            }
            // The type of an un-typeable node: compatible with everything, so it cannot cascade.
            Resolved::Poison(_) => Ty::Any,
        }
    }

    /// Collect the type-agreement faults reachable in a definition body — the type-error query. An
    /// `if` whose condition is not `Bool`, or whose branches do not agree, is a coded mismatch. This
    /// is a READ over the already-filled type column (it demands `type_of` on the nodes it checks),
    /// separate from the value the column holds, so filling a type never rejects and checking never
    /// re-derives a type (`reference-compiler.md` §A Meaning-Preserving Rewrite Preserves Value And
    /// Checks; §Types Are Solved Once).
    pub fn type_errors(&mut self, id: StructId) -> Vec<Reject> {
        let mut out = Vec::new();
        self.collect_type_errors(id, &mut out);
        out
    }

    fn collect_type_errors(&mut self, id: StructId, out: &mut Vec<Reject>) {
        if let Resolved::If { cond, then_, else_ } = self.resolved_of(id) {
            let cond_ty = self.type_of(cond);
            if !cond_ty.agrees_with(&Ty::Bool) {
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!("if condition must be Bool, found {}", cond_ty.render_name()),
                ));
            }
            let then_ty = self.type_of(then_);
            let else_ty = self.type_of(else_);
            if !then_ty.agrees_with(&else_ty) {
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "if branches differ: {} vs {}",
                        then_ty.render_name(),
                        else_ty.render_name()
                    ),
                ));
            }
            // Descend into the children (their own faults).
            self.collect_type_errors(cond, out);
            self.collect_type_errors(then_, out);
            self.collect_type_errors(else_, out);
        }
    }

    // ── The core column ──────────────────────────────────────────────────────────────────────

    /// The core (A-normal) form of the node at `id`, filling the column on demand. Reads the resolved
    /// form (and, implicitly through the backend/eval, the type column). Per-node: children stay ids.
    pub fn core_of(&mut self, id: StructId) -> Core {
        if let Slot::Filled(c) = self.core.get(id) {
            return c.clone();
        }
        let c = self.compute_core(id);
        self.core.fill(id, c.clone());
        c
    }

    /// Lower one node's resolved form to its core form. Stage 0's slice is atomic, so this is close
    /// to a structural map; children remain AST ids, lowered on their own demand.
    fn compute_core(&mut self, id: StructId) -> Core {
        match self.resolved_of(id) {
            Resolved::Int(v) => Core::ConstInt(v),
            Resolved::Bool(b) => Core::ConstBool(b),
            Resolved::Unit => Core::Unit,
            Resolved::If { cond, then_, else_ } => Core::If { cond, then_, else_ },
            Resolved::Poison(r) => Core::Poison(r),
        }
    }
}

/// The one cheap top-level scan: gather the definitions and export requests from the top form only,
/// without entering any body. Recognizes `(module NAME item…)`, a bare `(do item…)`, or a lone item.
fn scan_top_level(ast: &Arenas) -> (Vec<Def>, Vec<Export>) {
    let mut defs: Vec<Def> = Vec::new();
    let mut exports: Vec<Export> = Vec::new();

    for item in top_items(ast) {
        if let Some(tail) = ast.as_form(item, "def") {
            // `(def (NAME param…) BODY)`.
            let (name, params) = match tail.first().map(|&s| (s, ast.get(s))) {
                Some((_, Struct::List(children))) if !children.is_empty() => {
                    let name = ast.as_name(children[0]).unwrap_or("").to_string();
                    (name, children[1..].to_vec())
                }
                _ => (String::new(), Vec::new()),
            };
            let sig_occ = tail.first().copied().unwrap_or(item);
            let body = tail.get(1).copied();
            defs.push(Def { name, sig_occ, params, body });
        } else if let Some(tail) = ast.as_form(item, "export") {
            if let Some(name) = tail.first().and_then(|&s| ast.as_name(s)) {
                exports.push(Export { name: name.to_string(), def: None });
            }
        }
    }

    // Resolve each export's target index by name against the gathered defs (a signature read, not a
    // body read).
    let mut i = 0;
    while i < exports.len() {
        let target = defs.iter().position(|d| d.name == exports[i].name);
        exports[i].def = target;
        i += 1;
    }

    (defs, exports)
}

/// The top-level item occurrences: the tail of `(module NAME …)` (past the name), the tail of
/// `(do …)`, or the root as a single item.
fn top_items(ast: &Arenas) -> Vec<StructId> {
    let root = ast.root;
    if let Some(tail) = ast.as_form(root, "module") {
        return tail.get(1..).unwrap_or(&[]).to_vec();
    }
    if let Some(tail) = ast.as_form(root, "do") {
        return tail.to_vec();
    }
    vec![root]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Builder, IntValue, Leaf};

    /// Build `(module m (def (main) 42) (export main))` and return `(arenas, body-node-id)`.
    fn scalar_program() -> (Arenas, StructId) {
        let mut b = Builder::new();
        let module = b.name("module");
        let m = b.name("m");
        // (def (main) 42)
        let def = b.name("def");
        let main_sig_name = b.name("main");
        let sig = b.list(vec![main_sig_name]);
        let body = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(42), radix: crate::ast::Radix::Dec });
        let def_form = b.list(vec![def, sig, body]);
        // (export main)
        let export = b.name("export");
        let main_ref = b.name("main");
        let export_form = b.list(vec![export, main_ref]);
        let root = b.list(vec![module, m, def_form, export_form]);
        let ast = b.finish(root);
        (ast, body)
    }

    #[test]
    fn scan_finds_def_and_export() {
        let (ast, _) = scalar_program();
        let db = Db::load(ast);
        assert_eq!(db.defs.len(), 1);
        assert_eq!(db.defs[0].name, "main");
        assert!(db.defs[0].params.is_empty());
        assert_eq!(db.exports.len(), 1);
        assert_eq!(db.exports[0].name, "main");
        assert_eq!(db.exports[0].def, Some(0));
    }

    #[test]
    fn type_of_a_literal_is_a_deferred_int_rendering_as_int64() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        // The type of the literal node is a column read — the Stage-0 done-criterion. A bare literal
        // has a DEFERRED width (nothing has fixed it), which agrees with the concrete `Int64` and
        // renders as `Int64` (the default the observed value takes).
        let t = db.type_of(body);
        assert_eq!(t, Ty::int());
        assert!(t.agrees_with(&Ty::int64()));
        assert_eq!(t.render_name(), "Int64");
    }

    #[test]
    fn resolved_and_core_of_the_literal() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        assert_eq!(db.resolved_of(body), Resolved::Int(IntValue::from_i64(42)));
        assert_eq!(db.core_of(body), Core::ConstInt(IntValue::from_i64(42)));
    }

    #[test]
    fn querying_one_node_does_not_fill_unrelated_columns() {
        // Laziness: asking for the resolved form of the body must NOT have solved its type. A column
        // is filled only by its own demand.
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        let _ = db.resolved_of(body);
        assert!(matches!(db.types.get(body), Slot::Absent), "type column filled without demand");
        // Now demand the type; the slot fills.
        let _ = db.type_of(body);
        assert!(matches!(db.types.get(body), Slot::Filled(_)));
    }
}
