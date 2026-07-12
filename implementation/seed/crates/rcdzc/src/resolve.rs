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
use tracing::trace;

/// The fixed, closed set of GRAMMAR head NAMES — the forms that bind names or control evaluation, plus
/// member access. This set does NOT grow when a built-in value is added (that is a prelude entry).
/// A form whose head is one of these is dispatched structurally; any other NAME head is an application
/// (or, for a bare atom, a name looked up).
///
/// The compound-value constructors are NOT here: their PRIMITIVE is a STRING-LITERAL head — `("tuple"
/// …)` builds a tuple, `("record" …)` builds a record (dispatched via [`Arenas::head_ctor`], before
/// this name dispatch). A string is unspellable as an identifier, so the primitive can never be
/// shadowed; the ORDINARY names `tuple` and `record` are prelude ALIASES (shadowable) that reduce to
/// the same value. So a `(tuple …)` NAME head resolves through the ordinary scope→def→prelude lookup —
/// a local binding of `tuple` wins (the head-vs-value split that made `(let ((tuple …)) (tuple 3 4))`
/// ignore the binding is gone). See `core-semantics.md` §A Compound Value Has A Symbol Constructor And
/// A Shadowable Alias. ("The strings are the symbols" — the reserved primitive names are string
/// literals, needing no invented sigils and no reader change.)
const GRAMMAR: &[&str] = &[
    "let",
    "if",
    "match",
    ".",
    "module",
    "def",
    "export",
    "do",
    "unrealized",
    "intrinsic",
    "meta",
    "fn",
    "typeval",
    ":",
];

/// Whether `head` is a recognized GRAMMAR head — a binding/control/declaration form the resolver
/// dispatches structurally. A form whose head is NEITHER grammar NOR a bound value is an unmodeled
/// construct; `Db::unknown_top_forms` uses this to decline a top-level `(effect …)`/`(pragma …)` rather
/// than silently ignore it.
pub fn is_grammar_head(head: &str) -> bool {
    GRAMMAR.contains(&head)
}

/// The resolved form of the node at `id`, filling the column on demand (memoized). The query the
/// resolved-form request answers, and the upstream read `infer`/`lower` perform.
pub fn resolved_of(db: &mut Db, id: StructId) -> Resolved {
    if let Slot::Filled(r) = db.resolved.get(id) {
        trace!(target: "rcdzc::resolve", node = id.0, "memo hit");
        return r.clone();
    }
    let mut r = compute(db, id);
    // Anchor a resolver "no" to THIS node if it carries none — an unbound name or a malformed form is
    // about the node being resolved, so its diagnostic points there. (The ABI edge later drops the
    // anchor if this is a prelude/synthesized node rather than a user one.)
    if let Resolved::Poison(reject) = &mut r {
        reject.set_origin_if_absent(id);
    }
    trace!(target: "rcdzc::resolve", node = id.0, resolved = ?r, "resolved");
    db.resolved.fill(id, r.clone());
    r
}

/// Eagerly resolve an ENTIRE subtree, memoizing every node against its CURRENT lexical position. Used
/// to PIN a call-site argument's meaning before β-reduction splices it into a copied callee body: the
/// splice re-parents the argument's root (so the copied body's own names resolve against the copy —
/// necessary for a body-local `let`/param that binds a substituted value), which would otherwise drag
/// the argument out of the caller's scope and leave a caller-bound name (`(let ((k 10)) (inc k))`)
/// spuriously unbound. Resolving the argument here fills the memo, so the later re-parent cannot change
/// how any node inside it resolves — the "arguments resolve in the caller's scope" invariant
/// `apply_lambda` documents, made robust to the re-parenting `push_list` performs.
pub fn resolve_subtree(db: &mut Db, id: StructId) {
    resolved_of(db, id);
    let children = match db.ast.get(id) {
        Struct::List(children) => children.clone(),
        Struct::Atom(_) => return,
    };
    for c in children {
        resolve_subtree(db, c);
    }
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
                    trace!(target: "rcdzc::resolve", node = id.0, %n, "name → parameter (formal)");
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
            // The compound-value-constructor PRIMITIVES are STRING-LITERAL heads: `("record" …)` builds
            // a record, `("tuple" …)` builds a tuple. A string is unspellable as an identifier, so the
            // primitive is unshadowable — dispatched here, before the NAME dispatch. The ordinary names
            // `tuple`/`record` are prelude ALIASES that reduce to these (via `(meta apply)`), so they
            // are NOT matched below: a `(tuple …)` NAME head falls through to `Resolved::Apply` and
            // resolves lexically-first (a local `tuple` binding shadows the alias). ("The strings are
            // the symbols.")
            match db.ast.head_ctor(id) {
                Some("record") => return resolve_record(db, id),
                Some("tuple") => return resolve_tuple(db, id),
                _ => {}
            }
            match db.ast.head_name(id) {
                Some("if") => resolve_if(db, id),
                Some("match") => resolve_match(db, id),
                Some("let") => resolve_let(db, id),
                Some(".") => resolve_member(db, id),
                Some("fn") => resolve_lambda(db, id),
                Some(":") => resolve_annot(db, id),
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
                    Resolved::Poison(Reject::decline(format!(
                        "built-in `{op}` is not yet realized"
                    )))
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
                        Some(op) => {
                            trace!(target: "rcdzc::resolve", node = id.0, ?op, "intrinsic → prim");
                            Resolved::Prim(op)
                        }
                        None => {
                            trace!(target: "rcdzc::resolve", node = id.0, ?name, "unknown intrinsic (decline)");
                            Resolved::Poison(Reject::decline("unknown intrinsic"))
                        }
                    }
                }
                // A grammar declaration form appearing in expression position is not an expression:
                // `module`/`def`/`export`/`do`/`type` are recognized by the top-level scan
                // (`db::scan_top_level`), not by the expression resolver, so one here declines.
                Some(h) if GRAMMAR.contains(&h) => {
                    trace!(target: "rcdzc::resolve", node = id.0, head = %h, "grammar form in expression position (decline)");
                    Resolved::Poison(Reject::decline(format!("`{h}` is not an expression here")))
                }
                // A non-grammar head is an APPLICATION — always `Resolved::Apply`. An application is
                // structurally an application regardless of what the head turns out to be; WHETHER the
                // head is applyable is decided downstream by projecting its `(meta apply)`
                // (`eval::meta_apply_of`), a non-applyable head becoming a poison there. Resolve does
                // not pre-judge the head, so the one application path stays uniform and its dispatch
                // lives in one place (the meta channel), never the head's spelling.
                Some(_) | None => {
                    trace!(target: "rcdzc::resolve", node = id.0, head = children[0].0, args = children.len() - 1, "application");
                    Resolved::Apply {
                        head: children[0],
                        args: children[1..].to_vec(),
                    }
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
    // 1. Lexical scope — nearest enclosing binder. A binder yields a `Ref` to its value occurrence
    // (a `let`/param/scalar-match binder) OR a `SumPayload` (a variant-pattern binder binds the sum's
    // payload, not a plain occurrence) — `binder_in` returns the full resolved form.
    if let Some(resolved) = lookup_scope(db, id, name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, "name → lexical scope");
        return resolved;
    }
    // 2. The module's own top-level definitions. A nullary def denotes its body; a def WITH parameters
    // denotes a lambda `(params) body` (so a call `(f a)` applies it by the ordinary application
    // path). This is what makes a top-level function callable by name — and, being resolved lazily
    // per reference, a forward/mutual reference resolves regardless of definition order.
    if let Some(d) = db.def_by_name(name) {
        let def = &db.defs[d];
        trace!(target: "rcdzc::resolve", node = id.0, %name, params = def.params.len(), "name → top-level def");
        return match def.body {
            Some(body) if def.params.is_empty() => Resolved::Ref { value: body },
            Some(body) => Resolved::Lambda {
                params: def.params.clone().into(),
                body,
            },
            None => Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("`{name}` has no body"),
            )),
        };
    }
    // 3. The module's own SUM declarations — `(type NAME …)` binds `NAME` to its synthesized record
    // (fields = variants), resolved EXACTLY like a top-level def (step 2): a lookup against the
    // occurrence-keyed `type_decls`, returning a `Ref` to the record. After defs (a def/local shadows a
    // type name) and before the prelude (a type name shadows a built-in). So `Option`, `Option.Some`,
    // and `(: x Option)` all take the ordinary member-access/`(meta t)` paths — no separate name map,
    // no sum special-case in resolve.
    if let Some(value) = db.type_decl_by_name(name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → sum type decl");
        return Resolved::Ref { value };
    }
    // 4. The prelude map — a built-in binds to its installed arena node (a record, for a module). The
    // same `Ref` a program binding produces, so member access / folding treats it identically.
    if let Some(&value) = db.prelude.get(name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → prelude");
        return Resolved::Ref { value };
    }
    // 3. Off the end of the lookup — the name is unbound. This is a REJECTION (the program is
    // ill-formed), not a decline: the unbound-name rule is unconditional (`core-semantics.md`
    // §Binding Is Lexical).
    //
    // BUT a DIGIT-LED token is a NUMBER, never an identifier (an identifier may not start with a
    // digit). The reader classifies a numeric token that fails to parse — `0o17` (octal is not a
    // supported radix), `0x`/`0b` (empty radix body), `0xGG`/`0b12` (bad radix digit), `123abc`
    // (digits then letters) — as a bare `Leaf::Name` (the reader is minimal; the well-formedness call
    // is made here). Reporting such a token as "unbound name" (CDZ0101) is the misleading diagnostic
    // 01-literals.sexp explicitly forbids: it is a MALFORMED LITERAL, a well-formedness rejection
    // (CDZ0201), not a reference to a name. So a digit-led unbound name is Malformed, not Unbound.
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, "digit-led token is a MALFORMED literal (CDZ0201)");
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!("malformed numeric literal `{name}`"),
        ));
    }
    trace!(target: "rcdzc::resolve", node = id.0, %name, "name UNBOUND (CDZ0101)");
    Resolved::Poison(Reject::coded(
        Code::Unbound,
        format!("unbound name `{name}`"),
    ))
}

/// Walk parents from `id` to the nearest enclosing binding of `name`, returning the value occurrence
/// it is bound to. Nearest-wins gives shadowing for free (`core-semantics.md` §Binding Is Lexical,
/// §Shadowing Is Well-Defined). A `let` binds a name for the initializers that FOLLOW it and its body;
/// walking up from a reference, the first binder of `name` at or outside the reference's position is
/// the one in effect.
fn lookup_scope(db: &Db, id: StructId, name: &str) -> Option<Resolved> {
    let mut child = id;
    let mut cursor = db.parent_of(id);
    while let Some(form) = cursor {
        if let Some(resolved) = binder_in(db, form, child, name) {
            return Some(resolved);
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
/// A `def`'s parameters bind in its body too — Case 4 below, exactly as a `fn` body sees its own
/// parameters (a def-with-params is no longer nullary-only).
fn binder_in(db: &Db, form: StructId, from: StructId, name: &str) -> Option<Resolved> {
    // Case 1: `form` is a `let`, ascended from its body → all bindings visible.
    if let Some(tail) = db.ast.as_form(form, "let") {
        let bindings_occ = tail.first().copied()?;
        let body_occ = tail.get(1).copied();
        if Some(from) == body_occ {
            // The body sees EVERY binding — no `stop_before`.
            return last_binder_named(db, bindings_occ, name, None)
                .map(|v| Resolved::Ref { value: v });
        }
        return None;
    }
    // Case 2: `form` is a let's BINDINGS-LIST, ascended from pair `from` → the bindings BEFORE `from`
    // are visible (an initializer sees the earlier bindings, not itself or later ones).
    if let_of_bindings_list(db, form).is_some() {
        return last_binder_named(db, form, name, Some(from)).map(|v| Resolved::Ref { value: v });
    }
    // Case 3: `form` is a `(fn (params) body)`, ascended from the body → its parameters bind. A
    // parameter reference resolves to the PARAMETER occurrence itself (a formal); the evaluator
    // substitutes the argument there at application, and `infer` gives an unsubstituted parameter a
    // fresh variable. The last param of `name` wins (shadowing among params, harmless).
    if let Some(tail) = db.ast.as_form(form, "fn") {
        let params_occ = tail.first().copied()?;
        let body_occ = tail.get(1).copied();
        if Some(from) == body_occ
            && let Struct::List(params) = db.ast.get(params_occ)
        {
            // A parameter binds the name it declares — bare `a` or annotated `(: a T)` alike.
            let mut found = None;
            for &p in params {
                if let Some((n, name_occ)) = param_name(db, p)
                    && n == name
                {
                    found = Some(name_occ);
                }
            }
            return found.map(|v| Resolved::Ref { value: v });
        }
        return None;
    }
    // Case 4: `form` is a `(def (NAME param…) body)`, ascended from the body → the signature's
    // parameters (everything after NAME) bind. So a def-with-params body sees its parameters, exactly
    // as a lambda body sees its own. A parameter may be an annotated binder `(: a T)`.
    if let Some(tail) = db.ast.as_form(form, "def") {
        let sig_occ = tail.first().copied()?;
        let body_occ = tail.get(1).copied();
        if Some(from) == body_occ
            && let Struct::List(sig) = db.ast.get(sig_occ)
        {
            // sig = [NAME, param…]; a reference binds to the matching PARAM's name occurrence.
            let mut found = None;
            for &p in sig.iter().skip(1) {
                if let Some((n, name_occ)) = param_name(db, p)
                    && n == name
                {
                    found = Some(name_occ);
                }
            }
            return found.map(|v| Resolved::Ref { value: v });
        }
        return None;
    }
    // Case 5: `form` is a MATCH ARM `(pattern body)`, ascended from `body`, and `pattern` is a bare
    // BINDER name (not a literal, not `_`) equal to `name` → the binder binds the whole scrutinee for
    // this arm's body (`core-semantics.md` §Bindings Introduced By A Pattern Are Scoped To Its Branch).
    // The bound value IS the scrutinee, so a reference resolves to the scrutinee occurrence — its type
    // and (at lowering) its value flow straight through, no separate slot. Scoped to THIS arm only:
    // an arm is reached from the enclosing `(match …)`, so a binder in one arm is invisible to another.
    if let Some(scrutinee) = match_arm_binds(db, form, from, name) {
        return Some(Resolved::Ref { value: scrutinee });
    }
    // Case 6: `form` is a MATCH ARM whose pattern is a VARIANT pattern `((. Sum V) binder)`, ascended
    // from `body`, and `binder` is `name` → the binder binds the variant's PAYLOAD (not the whole
    // scrutinee, unlike Case 5). It resolves to a `SumPayload` reading `sum-payload(scrutinee)`; its
    // type is the variant's payload type. Scoped to this arm.
    if let Some((scrutinee, variant_head)) = match_arm_variant_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            variant_head,
        });
    }
    None
}

/// If `form` is a match ARM `(pattern body)` whose parent is a `(match scrutinee arm…)`, ascended from
/// the arm's `body`, and `pattern` is a bare BINDER name equal to `name` (not a literal, not the
/// wildcard `_`), the enclosing match's SCRUTINEE occurrence — the value the binder binds. `None`
/// otherwise. A binder pattern binds the whole scrutinee for its arm's body; the value is the
/// scrutinee, so a reference resolves straight to it.
fn match_arm_binds(db: &Db, form: StructId, from: StructId, name: &str) -> Option<StructId> {
    // `form` must be a 2-element `(pattern body)` list we ascended from its BODY (second element).
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 || pb[1] != from {
        return None;
    }
    let pattern = pb[0];
    // The pattern must be a bare binder name — matching `name`, and NOT a literal or the wildcard `_`
    // (those bind nothing). A binder is a plain `Name` other than `_`.
    let pat_name = db.ast.as_name(pattern)?;
    if pat_name != name || pat_name == "_" {
        return None;
    }
    // `form`'s parent must be a `(match scrutinee arm…)`, and `form` one of its arms (not the
    // scrutinee, which is the first tail element).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None; // `form` is the scrutinee position, not an arm
    }
    Some(scrutinee)
}

/// If `form` is a match ARM `(pattern body)` (ascended from its BODY) whose pattern is a VARIANT
/// pattern `((. Sum V) binder)` binding `name`, return `(scrutinee, variant_head)` — the enclosing
/// match's scrutinee occurrence and the pattern's variant-constructor occurrence `(. Sum V)`. The
/// binder binds the variant's PAYLOAD (via `sum-payload(scrutinee)`), so a reference resolves to a
/// `SumPayload`. `None` otherwise. Stage-5 handles a SINGLE bare binder as the payload; a destructuring
/// payload pattern (`(NPrim (tuple h a b))`) is a later increment.
fn match_arm_variant_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, StructId)> {
    // `form` = `(pattern body)`, ascended from body.
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 || pb[1] != from {
        return None;
    }
    let pattern = pb[0];
    // The pattern must be an application `(variant_head binder)`: a 2-element list whose head is a
    // MEMBER access `(. Sum V)` (a variant constructor) and whose sole argument is the bare binder
    // `name`. (A bare-member nullary pattern `(. Sum V)` binds nothing — not this case.)
    let Struct::List(app) = db.ast.get(pattern) else {
        return None;
    };
    if app.len() != 2 {
        return None;
    }
    let variant_head = app[0];
    // The head is the variant CONSTRUCTOR — either a `(. Sum V)` member access (`Option.Some`) OR a
    // bare variant NAME (`Some`, a prelude or in-scope ctor). Both are valid pattern heads; a head that
    // is neither (a bare-literal head, an arbitrary form) is not a variant pattern.
    let head_ok =
        db.ast.as_form(variant_head, ".").is_some() || db.ast.as_name(variant_head).is_some();
    if !head_ok {
        return None;
    }
    // The argument must be the bare binder `name` (not a literal, not `_`).
    let arg_name = db.ast.as_name(app[1])?;
    if arg_name != name || arg_name == "_" {
        return None;
    }
    // `form`'s parent must be a `(match scrutinee arm…)`, with `form` an arm (not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    Some((scrutinee, variant_head))
}

/// The NAME a parameter occurrence binds, and the occurrence that name lives at — seeing through a
/// `(: name T)` annotated binder. A parameter in a `fn`/`def` signature is EITHER a bare name (`a`) or
/// an annotated binder `(: a T)`; both bind `a`. Returns `(name, name-occurrence)`, where the
/// occurrence is the NAME's own node (so a reference binds to the name, and the annotation's type `T`
/// is read as the name's sibling — see `param_annot_ty`). `None` if the occurrence is neither shape.
fn param_name(db: &Db, param: StructId) -> Option<(&str, StructId)> {
    // A bare name parameter.
    if let Some(n) = db.ast.as_name(param) {
        return Some((n, param));
    }
    // An annotated binder `(: name T)` — bind the inner name; its type is `T`.
    if let Some(tail) = db.ast.as_form(param, ":") {
        let name_occ = *tail.first()?;
        let n = db.ast.as_name(name_occ)?;
        return Some((n, name_occ));
    }
    None
}

/// The value occurrence of the LAST binding named `name` visible from a lookup at this bindings-list,
/// or `None`. Last wins so a repeated binding shadows the earlier one.
///
/// A SINGLE allocation-free pass over the bindings-list children — the hot path of scope resolution.
/// (The earlier form materialized a `Vec<(StructId, StructId)>` of all pairs on EVERY name lookup;
/// resolving n references in one `let` re-scanned and re-allocated the whole list n times — O(n²)
/// with heavy malloc/free churn. This reads the children in place, in reverse, and returns on the
/// first match, so a lookup costs only the distance from the last matching binder.)
///
/// `stop_before` bounds the scope: `None` means the whole list is visible (a lookup from the `let`
/// BODY sees every binding); `Some(pair_occ)` means only the bindings BEFORE `pair_occ` are visible
/// (an initializer sees the earlier bindings, not itself or later ones — the Case-2 window). A
/// `stop_before` that names no pair in the list yields `None` (no binding is in scope).
fn last_binder_named(
    db: &Db,
    bindings_occ: StructId,
    name: &str,
    stop_before: Option<StructId>,
) -> Option<StructId> {
    let Struct::List(pairs) = db.ast.get(bindings_occ) else {
        return None;
    };
    // Find the window end: all pairs when `stop_before` is None, else the pairs strictly before it.
    // `from` is a child of `bindings_occ`, so its position is the precomputed `child_ix` — an O(1)
    // read rather than an O(k) scan of the sibling list (the difference between O(n) and O(n²) when
    // n references each ascend into an n-binding list).
    let end = match stop_before {
        None => pairs.len(),
        Some(from) => {
            let k = db.child_ix_of(from);
            // Defensive: the recorded position must actually address `from` in this list. (It always
            // does for a genuine child; a mismatch would mean `from` is not a child of `bindings_occ`,
            // in which case no binding is in scope.)
            if pairs.get(k) != Some(&from) {
                return None;
            }
            k
        }
    };
    // Scan the in-scope pairs in REVERSE and return the first match — the last binder wins, and a
    // reverse walk lets us stop as soon as we find it rather than scanning the whole prefix.
    for &pair in pairs[..end].iter().rev() {
        if let Struct::List(kv) = db.ast.get(pair)
            && kv.len() == 2
            && db.ast.as_name(kv[0]) == Some(name)
        {
            return Some(kv[1]);
        }
    }
    None
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

/// The `(binder-name-occ, init-occ)` pairs of a `let` bindings-list occurrence. A malformed pair is
/// skipped (it surfaces as a `Poison` when the let form is resolved).
fn binding_pairs(db: &Db, bindings_occ: StructId) -> Vec<(StructId, StructId)> {
    let mut out = Vec::new();
    if let Struct::List(pairs) = db.ast.get(bindings_occ) {
        for &pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair)
                && kv.len() == 2
            {
                out.push((kv[0], kv[1]));
            }
        }
    }
    out
}

/// Resolve `(if COND THEN ELSE)`.
fn resolve_if(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "if").unwrap_or(&[]);
    if tail.len() != 3 {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            "if takes exactly 3 operands",
        ));
    }
    Resolved::If {
        cond: tail[0],
        then_: tail[1],
        else_: tail[2],
    }
}

/// Resolve `(match SCRUTINEE (PATTERN BODY)…)` into its resolved form. The scrutinee and each arm's
/// pattern/body stay AST occurrences (resolved on their own demand); this records only the shape. Each
/// arm must be a two-element `(pattern body)` list. A `match` with no arms is malformed.
fn resolve_match(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "match").unwrap_or(&[]);
    let scrutinee = match tail.first() {
        Some(&s) => s,
        None => {
            return Resolved::Poison(Reject::coded(Code::Malformed, "match has no scrutinee"));
        }
    };
    let mut arms: Vec<(StructId, StructId)> = Vec::new();
    for &arm in &tail[1..] {
        match db.ast.get(arm) {
            Struct::List(pb) if pb.len() == 2 => arms.push((pb[0], pb[1])),
            _ => {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "a match arm must be (pattern body)",
                ));
            }
        }
    }
    if arms.is_empty() {
        return Resolved::Poison(Reject::coded(Code::Malformed, "match has no arms"));
    }
    Resolved::Match { scrutinee, arms }
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
    Resolved::Let {
        bindings: pairs,
        body,
    }
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
    if let Some(tail) = db.ast.as_form(node, "meta")
        && let Some(name) = tail.first().and_then(|&s| db.ast.as_name(s))
    {
        return Some(Symbol {
            namespace: Some("meta".to_string()),
            name: name.to_string(),
        });
    }
    None
}

/// Decode a `(typeval …)` payload node into the `Ty` it carries — the dual of `eval::encode_ty`. This
/// is an INTERNAL wire form the evaluator produces (like the binary codec), not user source, so
/// matching its tags is decoding a compiler-authored format, not source-name dispatch.
fn decode_ty(db: &Db, node: StructId) -> Option<crate::ty::Ty> {
    use crate::ty::{IntTy, Ty};
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
            Some(Ty::Int(IntTy::fixed(head == "Int", w)))
        }
        "->" => {
            let tail = db.ast.as_form(node, "->")?;
            let p = decode_ty(db, *tail.first()?)?;
            let r = decode_ty(db, *tail.get(1)?)?;
            Some(Ty::Fn(Box::new(p), Box::new(r)))
        }
        "Tuple" => {
            let tail = db.ast.as_form(node, "Tuple")?;
            let mut elems = Vec::with_capacity(tail.len());
            for &e in tail {
                elems.push(decode_ty(db, e)?);
            }
            Some(Ty::Tuple(elems.into()))
        }
        // A sum type-value: `(Sum <name> <decl> arg…)` — the dual of `eval::encode_ty`'s `Sum` arm (and
        // of the shape `sums::synthesize` builds for a sum record's `(meta t)`). The nominal name is for
        // rendering; the declaration occurrence (an integer literal) is the identity; the type ARGS
        // follow (empty for a monomorphic sum). Two sums are the same type iff their `decl` AND `args`
        // match (module A's `Foo` ≠ module B's `Foo`; `Option Int64` ≠ `Option Bool`).
        "Sum" => {
            let tail = db.ast.as_form(node, "Sum")?;
            let name = db.ast.as_name(*tail.first()?)?.to_string();
            let decl = match db.ast.get(*tail.get(1)?) {
                Struct::Atom(l) => match db.ast.leaf(*l) {
                    Leaf::Int { value, .. } => {
                        value.to_i64().and_then(|n| u32::try_from(n).ok())?
                    }
                    _ => return None,
                },
                _ => return None,
            };
            let mut args = Vec::new();
            for &a in tail.iter().skip(2) {
                args.push(decode_ty(db, a)?);
            }
            Some(Ty::Sum {
                decl: StructId(decl),
                name,
                args,
            })
        }
        // A record type-value: `(Record (name T)…)` — each `(name T)` a field pair. The head is
        // capitalized `Record` (the TYPE; the VALUE head is lowercase `record`), matching `encode_ty`
        // and the corpus type surface. The field-name SET + per-field types ARE the type.
        "Record" => {
            let tail = db.ast.as_form(node, "Record")?;
            let mut fields = std::collections::BTreeMap::new();
            for &pair in tail {
                let items = match db.ast.get(pair) {
                    Struct::List(items) if items.len() == 2 => items,
                    _ => return None,
                };
                let name = db.ast.as_name(items[0])?.to_string();
                let t = decode_ty(db, items[1])?;
                fields.insert(crate::resolved::Symbol::plain(name), t);
            }
            Some(Ty::Record(std::sync::Arc::new(fields)))
        }
        _ => None,
    }
}

/// Whether `id` is a lambda/def-parameter NAME occurrence — a formal, resolved to a `Param` rather
/// than looked up. `id` is such an occurrence when it is the name a parameter binds, whether the
/// parameter is a bare name (`id`'s parent is the param list) OR an annotated binder `(: id T)` (`id`
/// is the name-position of a `(:…)` whose parent is the param list). The type occurrence `T` inside a
/// binder is NOT a param occurrence — only the name is.
fn is_param_occurrence(db: &Db, id: StructId) -> bool {
    // `id` sits either directly in the param list (bare) or inside a `(: id T)` binder. Resolve the
    // node that appears IN the param list — `id` for a bare param, the `(:…)` node for an annotated
    // one — and remember whether `id` is that binder's name position.
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    let (param_node, list) = if db.ast.as_form(parent, ":").is_some() {
        // `id` is inside a `(: name T)`; it is a param NAME only if it is the name position (first),
        // never the type position. The binder node itself is what sits in the param list.
        let is_name_position =
            db.ast.as_form(parent, ":").and_then(|t| t.first().copied()) == Some(id);
        if !is_name_position {
            return false;
        }
        let Some(list) = db.parent_of(parent) else {
            return false;
        };
        (parent, list)
    } else {
        (id, parent)
    };
    let Some(form) = db.parent_of(list) else {
        return false;
    };
    // A `(fn (params) body)` parameter: `list` is the fn's parameter list (its first tail element).
    if let Some(tail) = db.ast.as_form(form, "fn")
        && tail.first().copied() == Some(list)
    {
        return true;
    }
    // A `(def (NAME param…) body)` parameter: `list` is the def's signature (its first tail element),
    // and `param_node` is NOT the signature's first element (that is the def NAME, not a parameter).
    if let Some(tail) = db.ast.as_form(form, "def")
        && tail.first().copied() == Some(list)
        && let Struct::List(sig) = db.ast.get(list)
    {
        return sig.first().copied() != Some(param_node);
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
        None => {
            return Resolved::Poison(Reject::coded(Code::Malformed, "fn has no parameter list"));
        }
    };
    let body = match tail.get(1) {
        Some(&b) => b,
        None => return Resolved::Poison(Reject::coded(Code::Malformed, "fn has no body")),
    };
    // The parameter occurrences (each a bare name). Collected into the `Arc<[StructId]>` the variant
    // holds (a refcounted slice — cloning the lambda is then O(1)).
    let params: std::sync::Arc<[StructId]> = match db.ast.get(params_occ) {
        Struct::List(ps) => ps.clone().into(),
        _ => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                "fn parameters must be a list",
            ));
        }
    };
    Resolved::Lambda { params, body }
}

/// Resolve `(record (k1 v1) (k2 v2) …)`. Field keys are labels (symbols, possibly `(meta …)`-
/// namespaced), NOT resolved. A duplicate field name makes the field set ill-defined → CDZ0201
/// (`core-semantics.md` §A Record Has A Fixed Set Of Named Fields); the check is over the WHOLE field
/// list, not adjacent pairs.
fn resolve_record(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_ctor_form(id, "record").unwrap_or(&[]);
    match read_record_fields(db, tail) {
        Ok(fields) => Resolved::Record {
            fields: std::sync::Arc::new(fields),
        },
        Err(reject) => Resolved::Poison(reject),
    }
}

/// Read a record's `(key value)` field list into the `label → value-occurrence` map — the ONE
/// field-reading rule, shared by the `{}` primitive (`resolve_record`) and the `record` alias
/// application (whose `(meta apply)` is `Prim::RecordNew`; `lower`/`infer` read the same fields). Each
/// field must be a two-element `(key value)` list; the key is a label via [`read_key`]; a duplicate
/// field name — anywhere in the list — is ill-formed. Returns the fault as `Err` so each caller wraps
/// it in the "no" shape it needs (a `Poison` value, a lowering poison, …).
pub(crate) fn read_record_fields(
    db: &Db,
    fields_tail: &[StructId],
) -> Result<BTreeMap<Symbol, StructId>, Reject> {
    let mut fields: BTreeMap<Symbol, StructId> = BTreeMap::new();
    for &field in fields_tail {
        let kv = match db.ast.get(field) {
            Struct::List(kv) if kv.len() == 2 => kv,
            _ => {
                return Err(Reject::coded(
                    Code::Malformed,
                    "record field must be (key value)",
                ));
            }
        };
        let label = match read_key(db, kv[0]) {
            Some(sym) => sym,
            None => {
                return Err(Reject::coded(
                    Code::Malformed,
                    "record field key must be a name or (meta name)",
                ));
            }
        };
        // A duplicate field name — anywhere in the list — is ill-formed.
        if fields.insert(label.clone(), kv[1]).is_some() {
            return Err(Reject::coded(
                Code::Malformed,
                format!("record names field `{}` more than once", label.name),
            ));
        }
    }
    Ok(fields)
}

/// Resolve `(. operand key)` — the ONE dotted projection form, whose KEY KIND selects the meaning:
/// an INTEGER-literal key is a positional TUPLE projection (`Proj` at that index — `(. t 0)`); a NAME
/// (or `(meta …)`) key is a named record/module member (`Member`). This is the "tuple access is just
/// an integer" surface (simpler than a `tuple.N` sigil; the one `.` form serves both). A key that is
/// neither an integer nor a label is a computed key, not yet supported (declines).
fn resolve_member(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, ".").unwrap_or(&[]);
    if tail.len() != 2 {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            "member access takes an operand and a key",
        ));
    }
    let operand = tail[0];
    // An INTEGER-literal key is a positional tuple projection. The index must be a non-negative integer
    // that fits a `usize` (a position); a negative or absurd index is malformed.
    if let Some(value) = db.ast.as_int(tail[1]) {
        return match tuple_index(value) {
            Some(index) => Resolved::Proj { operand, index },
            None => Resolved::Poison(Reject::coded(
                Code::Malformed,
                "a tuple index must be a non-negative position",
            )),
        };
    }
    let key = match read_key(db, tail[1]) {
        Some(sym) => sym,
        None => {
            return Resolved::Poison(Reject::decline(
                "a computed member key is not yet supported",
            ));
        }
    };
    Resolved::Member { operand, key }
}

/// A tuple index from an integer-literal key: a non-negative position that fits a `usize`, else `None`
/// (a negative index is not a position; an out-of-arity index is checked later against the operand's
/// static arity, where the tuple's type is known).
fn tuple_index(value: &crate::ast::IntValue) -> Option<usize> {
    if value.negative {
        return None;
    }
    value.to_i64().and_then(|n| usize::try_from(n).ok())
}

/// Resolve `(tuple e0 e1 …)` — a positional product literal. Every element is an AST occurrence in
/// order (resolved on demand); there is no key (positions are implicit). An empty `(tuple)` has no
/// elements — it is the empty product, which coincides with unit; but the reader writes `()` for unit,
/// so a written `(tuple)` is kept as a zero-element tuple here and typed as such (its arity is 0).
fn resolve_tuple(db: &Db, id: StructId) -> Resolved {
    let elems: std::sync::Arc<[StructId]> = db.ast.as_ctor_form(id, "tuple").unwrap_or(&[]).into();
    Resolved::Tuple { elems }
}

/// Resolve `(: expr ty_expr)` — a type annotation. Both children stay AST occurrences: `expr` is the
/// annotated value, `ty_expr` the type expression (reduced to a `Ty` downstream by the evaluator, not
/// here — resolve is a pure per-node classify, and reducing a type constructor like `(Int 8)` needs
/// `&mut Db`). The annotation is transparent to the value and constrains the type; that split is
/// realized by `infer` (unify) and `lower` (erase).
fn resolve_annot(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, ":").unwrap_or(&[]);
    if tail.len() != 2 {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            "a type annotation takes an expression and a type",
        ));
    }
    Resolved::Annot {
        expr: tail[0],
        ty_expr: tail[1],
    }
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
        assert_eq!(
            resolved_of(&mut db, body),
            Resolved::Int(IntValue::from_i64(42))
        );
    }

    #[test]
    fn resolves_an_if_leaving_children_as_ids() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        match resolved_of(&mut db, if_node) {
            Resolved::If { cond, then_, else_ } => {
                assert_eq!(resolved_of(&mut db, cond), Resolved::Bool(false));
                assert_eq!(
                    resolved_of(&mut db, then_),
                    Resolved::Int(IntValue::from_i64(1))
                );
                assert_eq!(
                    resolved_of(&mut db, else_),
                    Resolved::Int(IntValue::from_i64(2))
                );
            }
            other => panic!("expected If, got {other:?}"),
        }
    }
}
