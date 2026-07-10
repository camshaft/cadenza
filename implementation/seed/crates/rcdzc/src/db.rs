//! The database — the compiler's whole state as columns over one node identity. **Pure data.**
//!
//! This is the query engine's store (`query-engine.md`). There is ONE [`Db`]; it holds the decoded
//! AST, the top-level index (defs + exports) from one cheap scan, and a column per kind of derived
//! fact — the resolved form, the solved type, the core form — each keyed by the AST `StructId` that
//! is the node's identity throughout. It holds NO query logic: this file defines the data and how it
//! is loaded, and each *query* lives in its own module (`resolve`, `infer`, `lower`), a free function
//! over `&mut Db`. That separation is the nanopass "one pass owns one concern" discipline
//! (`reference-compiler.md` §One Pass Owns One Concern) expressed as one module per column, and it is
//! the shape the Cadenza port takes — `Resolve.resolved_of(db, id)`, `Infer.type_of(db, id)`, module
//! functions over a `Db` record.
//!
//! **How the columns are filled — the contract each query module keeps:**
//!  - Each column is filled by exactly ONE module: `resolve` fills `resolved`, `infer` fills `types`,
//!    `lower` fills `core`. A module reads another module's fact by calling that module's producer
//!    (e.g. `infer` calls `resolve::resolved_of`), NEVER by reading the raw column — which is also
//!    what makes the fill lazy: the producer fills on demand, so a raw read could see an empty slot.
//!  - A fact is answered by a backward demand that memoizes: the producer reads its column; on a miss
//!    it computes the answer (reading upstream facts through their producers) and fills its column.
//!    Asking one node's fact touches only the nodes that answer reaches; nothing is module-wide.
//!  - There is no cache separate from the columns and none to invalidate — incrementality is
//!    re-running a query (`query-engine.md` §Incrementality Is Re-Run, Not Invalidation).
//!
//! Absence is not a value: a slot is either filled or not, and a reader that requires a value and
//! finds absence declines rather than defaults (`query-engine.md` §A Reader That Requires A Value And
//! Finds Absence Declines). Every negative *decision* (a decline, a reject, a poison) is itself a
//! filled value — a `Resolved::Poison` / `Core::Poison` — so it is distinguished from "not yet
//! determined".

use crate::arena::Column;
use crate::ast::{Arenas, Struct, StructId};
use crate::core::Core;
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
/// columns. Constructed once per subject program; every query is a free function in a query module
/// that takes `&mut Db`. The columns are `pub(crate)` so a query module can fill its own and read the
/// memoized slot — the fill discipline (one module per column, read others via their producer) is the
/// contract documented above, not a visibility the type system enforces.
pub struct Db {
    /// The decoded AST — the source-of-truth column the others are derived from.
    pub ast: Arenas,
    /// The top-level definitions, from the one top-level scan (bounded; does not enter bodies).
    pub defs: Vec<Def>,
    /// The requested exports, from the scan.
    pub exports: Vec<Export>,

    /// For each `StructId`, the `List` occurrence that holds it as a child, or `None` for the root.
    /// The structure arena is NOT deduplicated, so every occurrence has exactly one parent — this is
    /// one deterministic scan, filled at load. It is what lets `resolve` derive a name's LEXICAL scope
    /// from the node's POSITION (walking parents to the nearest enclosing binder) rather than
    /// threading a scope argument that would break per-`StructId` memoization — the same
    /// provenance-by-back-reference the columns model uses for source position.
    parent: Vec<Option<StructId>>,

    /// The resolved-form column. Filled only by [`crate::resolve`].
    pub(crate) resolved: Column<StructId, Resolved>,
    /// The solved-type column. Filled only by [`crate::infer`].
    pub(crate) types: Column<StructId, Ty>,
    /// The core-form column. Filled only by [`crate::lower`].
    pub(crate) core: Column<StructId, Core>,
}

impl Db {
    /// Build a database over a decoded program: run the one cheap top-level scan (defs + exports),
    /// leave every derived column empty. Nothing below the top level is touched until a query demands
    /// it.
    pub fn load(ast: Arenas) -> Db {
        let (defs, exports) = scan_top_level(&ast);
        let parent = parent_index(&ast);
        Db {
            ast,
            defs,
            exports,
            parent,
            resolved: Column::new(),
            types: Column::new(),
            core: Column::new(),
        }
    }

    /// The `List` occurrence that holds `id` as a child, or `None` if `id` is the root — the one step
    /// of the lexical-scope walk.
    pub fn parent_of(&self, id: StructId) -> Option<StructId> {
        *self.parent.get(id.0 as usize).unwrap_or(&None)
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
}

/// Build the parent index: for each structure occurrence, the `List` occurrence that holds it as a
/// child (`None` for the root). One pass over the whole arena — deterministic (a child's parent is a
/// fixed function of the arena, no ordering or address involved).
fn parent_index(ast: &Arenas) -> Vec<Option<StructId>> {
    let mut parent = vec![None; ast.structure.len()];
    for i in 0..ast.structure.len() {
        if let Struct::List(children) = &ast.structure[i] {
            for &child in children {
                parent[child.0 as usize] = Some(StructId(i as u32));
            }
        }
    }
    parent
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
    use crate::testkit::scalar_program;

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
    fn def_by_name_finds_the_definition() {
        let (ast, _) = scalar_program();
        let db = Db::load(ast);
        assert_eq!(db.def_by_name("main"), Some(0));
        assert_eq!(db.def_by_name("nope"), None);
    }
}
