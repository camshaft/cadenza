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
use crate::resolved::{Prim, Resolved, Symbol};
use std::collections::BTreeMap;

/// The fixed, closed set of GRAMMAR head names — the forms that bind names or control evaluation, plus
/// member access. This set does NOT grow when a built-in value is added (that is a prelude entry).
/// A form whose head is one of these is dispatched structurally; any other head is an application (or,
/// for a bare atom, a name looked up).
const GRAMMAR: &[&str] = &[
    "let", "if", "record", ".", "module", "def", "export", "do", "unrealized", "intrinsic", "meta",
    "fn", "typeval",
];

/// The resolved form of the node at `id`, filling the column on demand (memoized). The query the
/// resolved-form request answers, and the upstream read `infer`/`lower` perform.
pub fn resolved_of(db: &mut Db, id: StructId) -> Resolved {
    if let Slot::Filled(r) = db.resolved.get(id) {
        return r.clone();
    }
    let mut r = compute(db, id);
    // Anchor a resolver "no" to THIS node if it carries none — an unbound name or a malformed form is
    // about the node being resolved, so its diagnostic points there. (The ABI edge later drops the
    // anchor if this is a prelude/synthesized node rather than a user one.)
    if let Resolved::Poison(reject) = &mut r {
        reject.set_origin_if_absent(id);
    }
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
            // A bare name. If this occurrence IS a lambda parameter (its parent is a `fn` param
            // list), it is a formal — a `Param`, not a lookup. Otherwise the one ordered lookup.
            Leaf::Name(n) => {
                if is_param_occurrence(db, id) {
                    Resolved::Param { binder: id }
                } else {
                    resolve_name(db, id, &n)
                }
            }
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
                Some("fn") => resolve_lambda(db, id),
                // `(typeval PAYLOAD)` — a built type-value node the evaluator produced; decode the
                // payload back to the `Ty` it carries. This is the dual of `eval::encode_typeval`.
                Some("typeval") => match db.ast.as_form(id, "typeval").and_then(|t| t.first()) {
                    Some(&payload) => match decode_ty(db, payload) {
                        Some(ty) => Resolved::TypeVal(ty),
                        None => Resolved::Poison(Reject::decline("malformed built type-value")),
                    },
                    None => Resolved::Poison(Reject::decline("malformed built type-value")),
                },
                // `(unrealized OP)` — a prelude field for an operation the compiler does not yet
                // realize. It resolves to a decline, so projecting it declines by the ordinary path
                // (no open-module rule). The op name rides along for the message.
                Some("unrealized") => {
                    let op = db
                        .ast
                        .as_form(id, "unrealized")
                        .and_then(|t| t.first())
                        .and_then(|&s| db.ast.as_name(s))
                        .unwrap_or("operation");
                    Resolved::Poison(Reject::decline(format!("built-in `{op}` is not yet realized")))
                }
                // `(intrinsic NAME)` — a prelude built-in operation VALUE. Resolves to the operation
                // it names (a value carried through the pipeline, lowered at selection).
                Some("intrinsic") => {
                    let name = db
                        .ast
                        .as_form(id, "intrinsic")
                        .and_then(|t| t.first())
                        .and_then(|&s| db.ast.as_name(s));
                    match name.and_then(Prim::from_name) {
                        Some(op) => Resolved::Prim(op),
                        None => Resolved::Poison(Reject::decline("unknown intrinsic")),
                    }
                }
                // A grammar declaration form appearing in expression position is not an expression
                // (Stage 0 handles module/def/export/do at the top level, not here).
                Some(h) if GRAMMAR.contains(&h) => {
                    Resolved::Poison(Reject::decline(format!("`{h}` is not an expression here")))
                }
                // A non-grammar head is an APPLICATION — always `Resolved::Apply`. An application is
                // structurally an application regardless of what the head turns out to be; WHETHER the
                // head is applyable is decided downstream by projecting its `(meta apply)`
                // (`eval::meta_apply_of`), a non-applyable head becoming a poison there. Resolve does
                // not pre-judge the head, so the one application path stays uniform and its dispatch
                // lives in one place (the meta channel), never the head's spelling.
                Some(_) | None => {
                    Resolved::Apply { head: children[0], args: children[1..].to_vec() }
                }
            }
        }
    }
}

/// Resolve a bare name at occurrence `id` by the one ordered lookup: the lexical scope, then the
/// module's own top-level definitions, then the prelude map (`prelude-and-resolution.md` §Name
/// Resolution Is One Ordered Lookup Returning The Bound Value). A hit is the value the name denotes;
/// scope-first order means a local binding SHADOWS a top-level def or built-in of the same name. A
/// miss is a `Poison`.
fn resolve_name(db: &Db, id: StructId, name: &str) -> Resolved {
    // 1. Lexical scope — nearest enclosing binder.
    if let Some(value) = lookup_scope(db, id, name) {
        return Resolved::Ref { value };
    }
    // 2. The module's own top-level definitions. A nullary def denotes its body; a def WITH parameters
    // denotes a lambda `(params) body` (so a call `(f a)` applies it by the ordinary application
    // path). This is what makes a top-level function callable by name — and, being resolved lazily
    // per reference, a forward/mutual reference resolves regardless of definition order.
    if let Some(d) = db.def_by_name(name) {
        let def = &db.defs[d];
        return match def.body {
            Some(body) if def.params.is_empty() => Resolved::Ref { value: body },
            Some(body) => Resolved::Lambda { params: def.params.clone(), body },
            None => Resolved::Poison(Reject::coded(Code::Malformed, format!("`{name}` has no body"))),
        };
    }
    // 3. The prelude map — a built-in binds to its installed arena node (a record, for a module). The
    // same `Ref` a program binding produces, so member access / folding treats it identically.
    if let Some(&value) = db.prelude.get(name) {
        return Resolved::Ref { value };
    }
    // 3. Off the end of the lookup — the name is unbound. This is a REJECTION (the program is
    // ill-formed), not a decline: the unbound-name rule is unconditional (`core-semantics.md`
    // §Binding Is Lexical).
    Resolved::Poison(Reject::coded(Code::Unbound, format!("unbound name `{name}`")))
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
    // Case 3: `form` is a `(fn (params) body)`, ascended from the body → its parameters bind. A
    // parameter reference resolves to the PARAMETER occurrence itself (a formal); the evaluator
    // substitutes the argument there at application, and `infer` gives an unsubstituted parameter a
    // fresh variable. The last param of `name` wins (shadowing among params, harmless).
    if let Some(tail) = db.ast.as_form(form, "fn") {
        let params_occ = tail.first().copied()?;
        let body_occ = tail.get(1).copied();
        if Some(from) == body_occ {
            if let Struct::List(params) = db.ast.get(params_occ) {
                let mut found = None;
                for &p in params {
                    if db.ast.as_name(p) == Some(name) {
                        found = Some(p);
                    }
                }
                return found;
            }
        }
        return None;
    }
    // Case 4: `form` is a `(def (NAME param…) body)`, ascended from the body → the signature's
    // parameters (everything after NAME) bind. So a def-with-params body sees its parameters, exactly
    // as a lambda body sees its own.
    if let Some(tail) = db.ast.as_form(form, "def") {
        let sig_occ = tail.first().copied()?;
        let body_occ = tail.get(1).copied();
        if Some(from) == body_occ {
            if let Struct::List(sig) = db.ast.get(sig_occ) {
                // sig = [NAME, param…]; a reference binds to the matching PARAM occurrence.
                let mut found = None;
                for &p in sig.iter().skip(1) {
                    if db.ast.as_name(p) == Some(name) {
                        found = Some(p);
                    }
                }
                return found;
            }
        }
        return None;
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

/// Read a KEY occurrence into a label [`Symbol`] — the one key-mode rule, shared by record fields and
/// member access. A key is EITHER a bare name → an unqualified `Symbol`, OR a `(meta NAME)` form → a
/// symbol in the `meta` namespace. The `(meta …)` form is how the meta channel is written as ordinary
/// structure (`((meta t) VALUE)` is a meta field; `(. r (meta t))` projects it), so a namespaced key
/// needs no dotted-string parsing and no reserved section — it is one more key shape read structurally
/// (`prelude-and-resolution.md` §A Member Key Is A Label). A key is NEVER resolved as a value.
fn read_key(db: &Db, node: StructId) -> Option<Symbol> {
    if let Some(n) = db.ast.as_name(node) {
        return Some(Symbol::plain(n));
    }
    // `(meta NAME)` → the symbol `NAME` in the `meta` namespace.
    if let Some(tail) = db.ast.as_form(node, "meta") {
        if let Some(name) = tail.first().and_then(|&s| db.ast.as_name(s)) {
            return Some(Symbol { namespace: Some("meta".to_string()), name: name.to_string() });
        }
    }
    None
}

/// Decode a `(typeval …)` payload node into the `Ty` it carries — the dual of `eval::encode_ty`. This
/// is an INTERNAL wire form the evaluator produces (like the binary codec), not user source, so
/// matching its tags is decoding a compiler-authored format, not source-name dispatch.
fn decode_ty(db: &Db, node: StructId) -> Option<crate::ty::Ty> {
    use crate::ty::{IntTy, Ty, Width};
    if let Some(name) = db.ast.as_name(node) {
        return match name {
            "Bool" => Some(Ty::Bool),
            "Unit" => Some(Ty::Unit),
            _ => None,
        };
    }
    let head = db.ast.head_name(node)?;
    match head {
        "Int" | "UInt" => {
            let tail = db.ast.as_form(node, head)?;
            let w = tail.first().and_then(|&s| match db.ast.get(s) {
                Struct::Atom(l) => match db.ast.leaf(*l) {
                    Leaf::Int { value, .. } => value.to_i64().and_then(|n| u32::try_from(n).ok()),
                    _ => None,
                },
                _ => None,
            })?;
            Some(Ty::Int(IntTy { signed: head == "Int", width: Width::Fixed(w) }))
        }
        "->" => {
            let tail = db.ast.as_form(node, "->")?;
            let p = decode_ty(db, *tail.first()?)?;
            let r = decode_ty(db, *tail.get(1)?)?;
            Some(Ty::Fn(Box::new(p), Box::new(r)))
        }
        _ => None,
    }
}

/// Whether `id` is a lambda-parameter occurrence — its parent is a list that is the parameter list of
/// an enclosing `(fn (params) body)` (i.e. that list is the `fn`'s first tail element). Such an
/// occurrence is a formal, resolved to a `Param` rather than looked up.
fn is_param_occurrence(db: &Db, id: StructId) -> bool {
    let Some(list) = db.parent_of(id) else { return false };
    let Some(form) = db.parent_of(list) else { return false };
    // A `(fn (params) body)` parameter: `list` is the fn's parameter list (its first tail element).
    if let Some(tail) = db.ast.as_form(form, "fn") {
        if tail.first().copied() == Some(list) {
            return true;
        }
    }
    // A `(def (NAME param…) body)` parameter: `list` is the def's signature (its first tail element),
    // and `id` is NOT the signature's first element (that is the def NAME, not a parameter).
    if let Some(tail) = db.ast.as_form(form, "def") {
        if tail.first().copied() == Some(list) {
            if let Struct::List(sig) = db.ast.get(list) {
                return sig.first().copied() != Some(id);
            }
        }
    }
    false
}

/// Resolve `(fn (param…) body)` into a compile-time lambda. The parameters bind in scope for `body`
/// (the ordinary parameter-scope mechanism, via `binder_in`). A type-lambda like `(fn (a) (-> (Int a)
/// …))` is just this — `a` is an ordinary parameter, not a special "type variable".
fn resolve_lambda(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "fn").unwrap_or(&[]);
    let params_occ = match tail.first() {
        Some(&p) => p,
        None => return Resolved::Poison(Reject::coded(Code::Malformed, "fn has no parameter list")),
    };
    let body = match tail.get(1) {
        Some(&b) => b,
        None => return Resolved::Poison(Reject::coded(Code::Malformed, "fn has no body")),
    };
    // The parameter occurrences (each a bare name).
    let params: Vec<StructId> = match db.ast.get(params_occ) {
        Struct::List(ps) => ps.clone(),
        _ => return Resolved::Poison(Reject::coded(Code::Malformed, "fn parameters must be a list")),
    };
    Resolved::Lambda { params, body }
}

/// Resolve `(record (k1 v1) (k2 v2) …)`. Field keys are labels (symbols, possibly `(meta …)`-
/// namespaced), NOT resolved. A duplicate field name makes the field set ill-defined → CDZ0201
/// (`core-semantics.md` §A Record Has A Fixed Set Of Named Fields); the check is over the WHOLE field
/// list, not adjacent pairs.
fn resolve_record(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "record").unwrap_or(&[]);
    let mut fields: BTreeMap<Symbol, StructId> = BTreeMap::new();
    for &field in tail {
        // Each field is `(key value)`.
        let kv = match db.ast.get(field) {
            Struct::List(kv) if kv.len() == 2 => kv,
            _ => return Resolved::Poison(Reject::coded(Code::Malformed, "record field must be (key value)")),
        };
        let label = match read_key(db, kv[0]) {
            Some(sym) => sym,
            None => return Resolved::Poison(Reject::coded(Code::Malformed, "record field key must be a name or (meta name)")),
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

/// Resolve `(. operand key)`. The key is a LABEL (bare or `(meta …)`), never resolved as a value.
/// A key that is neither is not yet supported (declines).
fn resolve_member(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, ".").unwrap_or(&[]);
    if tail.len() != 2 {
        return Resolved::Poison(Reject::coded(Code::Malformed, "member access takes an operand and a key"));
    }
    let operand = tail[0];
    let key = match read_key(db, tail[1]) {
        Some(sym) => sym,
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
