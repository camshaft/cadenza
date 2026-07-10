//! `resolve` — the query that fills the resolved column: for a node's `StructId`, what it denotes.
//!
//! One concern: meaning-of-a-name and shape-of-a-form. [`resolved_of`] classifies the AST occurrence
//! at one id and records its [`Resolved`] form, leaving children as ids for their own demand. It is a
//! per-node backward query that memoizes into `db.resolved`; it does not recurse and it touches no
//! other column. This is the ONLY module that fills `db.resolved`
//! (`reference-compiler.md` §One Pass Owns One Concern).
//!
//! ## Two generic operations over one map, plus a fixed grammar
//! The resolver recognizes a fixed, closed set of GRAMMAR forms by head name — the binding/control/
//! declaration forms and member access — and resolves every OTHER name through one ordered lookup:
//! the lexical scope, then the prelude map (`prelude-and-resolution.md` §The Names The Resolver Treats
//! As Grammar Are A Fixed, Closed Set). Adding a built-in never adds a grammar name — it is a prelude
//! entry. There is NO `if name == "…"` for a value: a bare name is looked up, never special-cased.
//!
//! ## Scope is derived from position, not threaded
//! A name's lexical scope is found by walking PARENTS (via `db.parent_of`) from the name's occurrence
//! to the nearest enclosing binder (`let` initializer/body, `def` parameter) that binds it. Deriving
//! scope from position — rather than passing a scope argument — is what keeps `resolved_of` a pure
//! function of a `StructId` (so its column memoizes), the same provenance-by-back-reference the
//! columns model uses for source position (`query-engine.md` §Provenance Is Recovered By
//! Back-Reference). A name resolves to a [`Resolved::Ref`] to its binding's value occurrence; an
//! unbound name is a `Poison`.
//!
//! ## A member key is a label
//! In `(. operand key)` the key is taken as a [`Symbol`] from its spelling — NO scope or prelude
//! lookup (`prelude-and-resolution.md` §A Member Key Is A Label, Not A Value). A record literal's
//! field names are labels the same way.
//!
//! It is TOTAL: an unrecognized head, a malformed form, an unbound name, or an unmodeled literal
//! becomes a [`Resolved::Poison`] rather than an abort.

use crate::arena::Slot;
use crate::ast::{Leaf, Struct, StructId};
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::resolved::{Resolved, Symbol};
use std::collections::BTreeMap;

/// The fixed, closed set of GRAMMAR head names — the forms that bind names or control evaluation, plus
/// member access. This set does NOT grow when a built-in value is added (that is a prelude entry).
/// A form whose head is one of these is dispatched structurally; any other head is an application (or,
/// for a bare atom, a name looked up).
const GRAMMAR: &[&str] = &["let", "if", "record", ".", "module", "def", "export", "do"];

/// The resolved form of the node at `id`, filling the column on demand (memoized). The query the
/// resolved-form request answers, and the upstream read `infer`/`lower` perform.
pub fn resolved_of(db: &mut Db, id: StructId) -> Resolved {
    if let Slot::Filled(r) = db.resolved.get(id) {
        return r.clone();
    }
    let r = compute(db, id);
    db.resolved.fill(id, r.clone());
    r
}

/// Classify one AST occurrence into its resolved form. Reads the AST + the parent index (for scope);
/// does not recurse into children (they resolve on their own demand). A "no" is a `Poison` value.
fn compute(db: &Db, id: StructId) -> Resolved {
    match db.ast.get(id) {
        Struct::Atom(leaf_id) => match db.ast.leaf(*leaf_id).clone() {
            // The literal's EXACT value flows through; its machine width is decided at select.
            Leaf::Int { value, .. } => Resolved::Int(value),
            Leaf::Bool(b) => Resolved::Bool(b),
            // A bare name: the one ordered lookup — lexical scope, then the prelude map.
            Leaf::Name(n) => resolve_name(db, id, &n),
            Leaf::Str(_) => Resolved::Poison(Reject::decline("string literals not yet supported")),
            Leaf::Float(_) => Resolved::Poison(Reject::decline("float literals not yet supported")),
        },
        Struct::List(children) => {
            // `()` — the empty list — is unit.
            if children.is_empty() {
                return Resolved::Unit;
            }
            match db.ast.head_name(id) {
                Some("if") => resolve_if(db, id),
                Some("let") => resolve_let(db, id),
                Some("record") => resolve_record(db, id),
                Some(".") => resolve_member(db, id),
                // A grammar declaration form appearing in expression position is not an expression
                // (Stage 0 handles module/def/export/do at the top level, not here).
                Some(h) if GRAMMAR.contains(&h) => {
                    Resolved::Poison(Reject::decline(format!("`{h}` is not an expression here")))
                }
                // A non-grammar head is an application — not yet realized (no user functions in
                // expression position in Stage 1).
                Some(h) => Resolved::Poison(Reject::decline(format!("application of `{h}` not yet supported"))),
                None => Resolved::Poison(Reject::decline("a form whose head is not a name")),
            }
        }
    }
}

/// Resolve a bare name at occurrence `id` by the one ordered lookup: the lexical scope (walk parents
/// to the nearest enclosing binder of `name`), then the prelude map. A hit is a `Ref` to the value the
/// name denotes; a miss is a `Poison` (unbound).
fn resolve_name(db: &Db, id: StructId, name: &str) -> Resolved {
    if let Some(value) = lookup_scope(db, id, name) {
        return Resolved::Ref { value };
    }
    // The prelude map is the next (and last) step of the lookup. Stage 1's prelude is empty of value
    // bindings (built-in modules arrive as entries here later); an unbound name declines.
    Resolved::Poison(Reject::decline(format!("unbound name `{name}`")))
}

/// Walk parents from `id` to the nearest enclosing binding of `name`, returning the value occurrence
/// it is bound to. Nearest-wins gives shadowing for free (`core-semantics.md` §Binding Is Lexical,
/// §Shadowing Is Well-Defined). A `let` binds a name for the initializers that FOLLOW it and its body;
/// walking up from a reference, the first binder of `name` at or outside the reference's position is
/// the one in effect.
fn lookup_scope(db: &Db, id: StructId, name: &str) -> Option<StructId> {
    let mut child = id;
    let mut cursor = db.parent_of(id);
    while let Some(form) = cursor {
        if let Some(value) = binder_in(db, form, child, name) {
            return Some(value);
        }
        child = form;
        cursor = db.parent_of(form);
    }
    None
}

/// If `form` binds `name` in scope for the child `from` we ascended from, the value occurrence bound.
/// Two binder levels of a `let`, so a reference finds exactly the bindings in scope where it sits:
///
///  - `form` is the `let` and `from` is the BODY → all bindings visible, last-binding-of-`name` wins
///    (a repeat shadows the earlier one — `core-semantics.md` §A Repeated Binding Shadows).
///  - `form` is the let's BINDINGS-LIST and `from` is pair k → bindings 0..k visible (binding k's
///    initializer sees the earlier bindings, not itself — §"A later let binding sees an earlier one").
///
/// A `def` parameter would bind in the body here too; Stage 1 defs are nullary, so nothing yet.
fn binder_in(db: &Db, form: StructId, from: StructId, name: &str) -> Option<StructId> {
    // Case 1: `form` is a `let`, ascended from its body → all bindings visible.
    if let Some(tail) = db.ast.as_form(form, "let") {
        let bindings_occ = tail.first().copied()?;
        let body_occ = tail.get(1).copied();
        if Some(from) == body_occ {
            let pairs = binding_pairs(db, bindings_occ);
            return last_binding_of(db, &pairs, name, pairs.len());
        }
        return None;
    }
    // Case 2: `form` is a let's BINDINGS-LIST, ascended from pair k → bindings 0..k visible.
    if let_of_bindings_list(db, form).is_some() {
        let pairs = binding_pairs(db, form);
        // `from` is one of the `(name init)` pair occurrences; its index k is how many earlier
        // bindings are in scope for this initializer.
        if let Some(k) = pair_index(db, form, from) {
            return last_binding_of(db, &pairs, name, k);
        }
    }
    None
}

/// The value occurrence of the LAST of the first `upto` bindings whose name is `name`, or `None`. Last
/// wins so a repeated binding shadows the earlier one.
fn last_binding_of(
    db: &Db,
    pairs: &[(StructId, StructId)],
    name: &str,
    upto: usize,
) -> Option<StructId> {
    let mut found = None;
    for &(binder_name_occ, init_occ) in pairs.iter().take(upto) {
        if db.ast.as_name(binder_name_occ) == Some(name) {
            found = Some(init_occ);
        }
    }
    found
}

/// If `form` is the bindings-list of a `let` (its parent is a `let` and `form` is that let's first
/// tail element), the enclosing `let` form.
fn let_of_bindings_list(db: &Db, form: StructId) -> Option<StructId> {
    let parent = db.parent_of(form)?;
    let tail = db.ast.as_form(parent, "let")?;
    if tail.first().copied() == Some(form) {
        Some(parent)
    } else {
        None
    }
}

/// The index of the binding pair (in `bindings_occ`) whose subtree we ascended from at `from` — i.e.
/// which `(name init)` pair `from` is.
fn pair_index(db: &Db, bindings_occ: StructId, from: StructId) -> Option<usize> {
    if let Struct::List(pairs) = db.ast.get(bindings_occ) {
        return pairs.iter().position(|&p| p == from);
    }
    None
}

/// The `(binder-name-occ, init-occ)` pairs of a `let` bindings-list occurrence. A malformed pair is
/// skipped (it surfaces as a `Poison` when the let form is resolved).
fn binding_pairs(db: &Db, bindings_occ: StructId) -> Vec<(StructId, StructId)> {
    let mut out = Vec::new();
    if let Struct::List(pairs) = db.ast.get(bindings_occ) {
        for &pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair) {
                if kv.len() == 2 {
                    out.push((kv[0], kv[1]));
                }
            }
        }
    }
    out
}

/// Resolve `(if COND THEN ELSE)`.
fn resolve_if(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "if").unwrap_or(&[]);
    if tail.len() != 3 {
        return Resolved::Poison(Reject::coded(Code::Malformed, "if takes exactly 3 operands"));
    }
    Resolved::If { cond: tail[0], then_: tail[1], else_: tail[2] }
}

/// Resolve `(let (BINDINGS) BODY)` into its resolved form. The bindings are `(name init)` pairs; scope
/// is handled by the parent-walk (a reference finds its binder), so here we only record the shape.
fn resolve_let(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "let").unwrap_or(&[]);
    let bindings_occ = match tail.first() {
        Some(&b) => b,
        None => return Resolved::Poison(Reject::coded(Code::Malformed, "let has no bindings")),
    };
    let body = match tail.get(1) {
        Some(&b) => b,
        None => return Resolved::Poison(Reject::coded(Code::Malformed, "let has no body")),
    };
    let pairs = binding_pairs(db, bindings_occ);
    if pairs.is_empty() {
        return Resolved::Poison(Reject::coded(Code::Malformed, "let bindings are malformed"));
    }
    Resolved::Let { bindings: pairs, body }
}

/// Resolve `(record (k1 v1) (k2 v2) …)`. Field names are labels (symbols), NOT resolved. A duplicate
/// field name makes the field set ill-defined → CDZ0201 (`core-semantics.md` §A Record Has A Fixed Set
/// Of Named Fields); the check is over the WHOLE field list, not adjacent pairs.
fn resolve_record(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "record").unwrap_or(&[]);
    let mut fields: BTreeMap<Symbol, StructId> = BTreeMap::new();
    for &field in tail {
        // Each field is `(name value)`.
        let kv = match db.ast.get(field) {
            Struct::List(kv) if kv.len() == 2 => kv,
            _ => return Resolved::Poison(Reject::coded(Code::Malformed, "record field must be (name value)")),
        };
        let label = match db.ast.as_name(kv[0]) {
            Some(n) => Symbol::plain(n),
            None => return Resolved::Poison(Reject::coded(Code::Malformed, "record field name must be a name")),
        };
        // A duplicate field name — anywhere in the list — is ill-formed.
        if fields.insert(label.clone(), kv[1]).is_some() {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("record names field `{}` more than once", label.name),
            ));
        }
    }
    Resolved::Record { fields }
}

/// Resolve `(. operand key)`. The key is a LABEL read from its spelling — never resolved as a value.
/// A computed key (a non-name key occurrence) is not yet supported (declines).
fn resolve_member(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, ".").unwrap_or(&[]);
    if tail.len() != 2 {
        return Resolved::Poison(Reject::coded(Code::Malformed, "member access takes an operand and a key"));
    }
    let operand = tail[0];
    let key = match db.ast.as_name(tail[1]) {
        Some(n) => Symbol::plain(n),
        None => return Resolved::Poison(Reject::decline("a computed member key is not yet supported")),
    };
    Resolved::Member { operand, key }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::IntValue;
    use crate::testkit::{if_program, scalar_program};

    #[test]
    fn resolves_a_literal() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        assert_eq!(resolved_of(&mut db, body), Resolved::Int(IntValue::from_i64(42)));
    }

    #[test]
    fn resolves_an_if_leaving_children_as_ids() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        match resolved_of(&mut db, if_node) {
            Resolved::If { cond, then_, else_ } => {
                assert_eq!(resolved_of(&mut db, cond), Resolved::Bool(false));
                assert_eq!(resolved_of(&mut db, then_), Resolved::Int(IntValue::from_i64(1)));
                assert_eq!(resolved_of(&mut db, else_), Resolved::Int(IntValue::from_i64(2)));
            }
            other => panic!("expected If, got {other:?}"),
        }
    }
}
