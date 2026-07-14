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
use crate::resolved::{HandleArm, Prim, Resolved, Symbol};
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
    // `quote` is a NON-STRICT form — it SUPPRESSES evaluation of its operand (yielding the operand's AST
    // rather than its value), control flow like `if`/`match`, NOT a strict prelude record (which would
    // evaluate the operand). Kept here so a `(quote …)` is an expression the resolver dispatches
    // structurally. This increment only rejects the DEGENERATE zero-operand `(quote)` (malformed,
    // CDZ0201); real quotation (building an `Ast` value) is the metaprogramming vertical.
    "quote",
    // `quasiquote` (`` ` ``) is the SELECTIVE-evaluation template + `unquote` (`,`) / `unquote-splicing`
    // (`,@`) its escapes — reader-desugared to `(quasiquote …)` / `(unquote …)` / `(unquote-splicing …)`.
    // Grammar (like `quote`), NOT prelude records: they SUPPRESS/select evaluation structurally. This
    // increment recognizes them to REJECT the two syntax defects (an `unquote`/`,@` OUTSIDE any quasiquote
    // → CDZ0003; a wrong-arity `unquote` → CDZ0201); a well-formed one inside a quasiquote DECLINES (the
    // `Ast`-construction vertical). Kept here so `(unquote …)` is not read as an unbound name / unmodeled
    // top-level form.
    "quasiquote",
    "unquote",
    "unquote-splicing",
    // `and`/`or`/`not` are SHORT-CIRCUITING boolean connectives — CONTROL FLOW (like `if`), not strict
    // value operators, so they are grammar, NOT prelude records (a strict `(meta apply)` record would
    // evaluate both operands, breaking the shield core-semantics.md §Boolean Connectives Short-Circuit
    // requires). `not` is strict but is a fixed one-operand form kept here with its siblings.
    "and",
    "or",
    "not",
    // `|>` is the PIPELINE operator — a SYNTACTIC form the resolver rewrites into an ordinary
    // application (`(|> L R)` threads `L` as `R`'s first argument), NOT a prelude value: threading
    // `L` into `R`'s argument list is a structural rewrite a strict `(meta apply)` record could not
    // express. Kept here with its control-flow siblings (like `.` member access) so a top-level
    // `(|> …)` is an expression, not an unknown declaration (`db::unknown_top_forms`).
    "|>",
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
    // Effect control forms — `handle` installs an in-program handler, `host` delegates to the boundary,
    // `resume` hands a value back inside a handler arm. Control flow (like `if`/`match`), reduced away by
    // the compile-time evaluator, NOT prelude records — so they are grammar (`DESIGN-effects-rcdzc.md`).
    // `handle` stays grammar so a leftover (non-canonical) one dispatches to its reject rather than
    // reading as an unmodeled top-level form; `handle-internal` is the desugared node the resolver folds.
    "handle",
    crate::effects::HANDLE_INTERNAL,
    "host",
    "resume",
];

/// Whether `head` is a recognized GRAMMAR head — a binding/control/declaration form the resolver
/// dispatches structurally. A form whose head is NEITHER grammar NOR a bound value is an unmodeled
/// construct; `Db::unknown_top_forms` uses this to decline a top-level `(effect …)`/`(pragma …)` rather
/// than silently ignore it.
pub fn is_grammar_head(head: &str) -> bool {
    GRAMMAR.contains(&head)
}

/// Whether the name occurrence `id` is the HEAD (first child) of a List whose head-name is a GRAMMAR
/// keyword — i.e. `id` is the literal `def`/`let`/`if`/`do`/… keyword TOKEN of a form the resolver
/// dispatches STRUCTURALLY by that head name. Such a token is a SYNTACTIC KEYWORD, not a value
/// reference: it never denotes a binding, and its enclosing form is resolved by `head_name`, which never
/// resolves the head atom itself. (An APPLICATION head `(f x)` is NOT caught — `f` is not a grammar
/// keyword — so it still resolves to a `Ref`, keeping application-head lookup and use-tracking intact.)
///
/// This exists because `compute` is occasionally asked to resolve a form-head atom DIRECTLY — the
/// module-wide `compile::collect_unused_binding_warnings` walk calls `resolved_of` on EVERY node — and a
/// grammar keyword falling through to `resolve_name` was an "unbound name", which then runs the O(scope)
/// `nearest_name_suggestion` typo scan (a Levenshtein per candidate). With one grammar keyword per `def`,
/// a module of N defs paid O(N²): profiling an N-def module showed ~90% of the whole compile in that
/// suggestion scan over the `def`/`do`/`export` head tokens. Recognizing the keyword position as INERT
/// (resolve to `Unit`, like the pattern-binder positions) removes the spurious lookup for every caller.
fn is_grammar_form_head_occurrence(db: &Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    // `id` must be the FIRST child of `parent`, and `parent`'s head name a grammar keyword. `head_name`
    // reads that same first child's name, so this holds exactly when `id` IS the grammar head token.
    matches!(db.ast.get(parent), Struct::List(kids) if kids.first() == Some(&id))
        && db.ast.head_name(parent).is_some_and(is_grammar_head)
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
    // Already fully walked? The walk is idempotent (a subtree, once resolved, stays resolved), so a
    // repeat is pure waste. `apply_lambda` re-pins the same/overlapping argument subtrees across a
    // chain of applications; without this guard the re-walk was O(N^2) on a shared-tuple projection
    // chain (`(+ (. t 0) (+ (. t 1) ...))`). See `Db::resolved_subtrees`.
    if db.resolved_subtrees.contains(&id) {
        return;
    }
    resolved_of(db, id);
    if let Struct::List(children) = db.ast.get(id) {
        for c in children.clone() {
            resolve_subtree(db, c);
        }
    }
    db.resolved_subtrees.insert(id);
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
                } else if is_list_pattern_element_occurrence(db, id) {
                    // A `(list a b)` match-pattern element binder IN THE PATTERN position — it names a
                    // binding, not a value. It carries nothing on its own (a body reference resolves to a
                    // `SumPayload` reading the scrutinee element, `binder_in` Case 6l); resolving the
                    // pattern occurrence to `Unit` keeps it INERT, so walking the arm's pattern (e.g.
                    // `resolve_subtree`) never reports it as a spurious unbound name.
                    trace!(target: "rcdzc::resolve", node = id.0, %n, "name → list-pattern element binder (inert)");
                    Resolved::Unit
                } else if is_map_pattern_binder_occurrence(db, id) {
                    // A `(map (k v) … .. rest)` match-pattern VALUE binder (`v`) or REST binder (`rest`) IN
                    // THE PATTERN position — a binding, not a value (a body reference resolves to a
                    // `MapField` reading the scrutinee, `binder_in` Case M). Inert here (like a list-pattern
                    // element binder), so walking the arm's pattern never reports it unbound. The KEY
                    // position `k` is NOT caught here — it is an ordinary value expression (a literal / a
                    // scoped name), so it resolves normally (the map-key-is-a-value rule, on the pattern side).
                    trace!(target: "rcdzc::resolve", node = id.0, %n, "name → map-pattern binder (inert)");
                    Resolved::Unit
                } else if is_grammar_form_head_occurrence(db, id) {
                    // A GRAMMAR keyword in head position (`def`/`let`/`do`/…): a syntactic keyword TOKEN,
                    // NOT a value reference. Its enclosing form is dispatched by `head_name`, which never
                    // resolves this atom; only a blanket node walk (the unused-binding warning pass) asks.
                    // Resolve it INERT (like a pattern binder) so it does NOT fall to `resolve_name` and
                    // run the O(scope) `nearest_name_suggestion` typo scan — which made an N-def module
                    // O(N²) (one keyword per def × an O(N) scan each). An application head is not a grammar
                    // keyword, so it is unaffected and still resolves to its `Ref`.
                    trace!(target: "rcdzc::resolve", node = id.0, %n, "name → grammar keyword head (inert)");
                    Resolved::Unit
                } else {
                    resolve_name(db, id, &n)
                }
            }
            // A string literal — its text is already unescaped to canonical form by the reader. A
            // `Ty::String` constant (folds to `Core::ConstStr`, escapes as its baked UTF-8 bytes).
            Leaf::Str(s) => Resolved::Str(s.clone()),
            // A SYMBOL literal (`#"metre"`) — the reader-sugar equivalent of `(Symbol.of "metre")`
            // (17-symbols). Resolves to a `SymbolConst` typed `Ty::Symbol` (DISTINCT from `Ty::String`, so
            // `(= #"x" "x")` is the nominal-boundary type error CDZ0202 and `(= #"x" (Symbol.of "x"))` is
            // true). Its identity is its text, so it shares the `Core::ConstStr` rep + constant-string
            // equality. `Unit.base #"metre"` still reads its text (a base-dimension name — `unit_of`
            // accepts this form). (Was a `Str` in the Layer-1 units simplification, before `Ty::Symbol`.)
            Leaf::Sym(s) => Resolved::SymbolConst(s.clone()),
            // A byte-string literal `b"…"` — the reader unescaped it to raw bytes. A `Ty::Bytes`
            // constant (lowers to a `Core::BytesOf` of its bytes, so it bakes/compares/slices exactly
            // like `(Bytes.of (list …))`, and renders back `b"…"`). The companion of the `Str` literal.
            Leaf::Bytes(bs) => Resolved::Bytes(bs.clone()),
            // A FLOAT literal — types as `Ty::Float`, distinct from `Ty::Int` (so an int↔float mix
            // rejects, no silent promotion). A literal whose magnitude exceeds the finite Float64 range
            // rounds to `±inf`, which has no written form the reader accepts — so it is a MALFORMED
            // literal (CDZ0201), the float analogue of the out-of-range integer literal
            // `9223372036854775808` (numeric-model.md §A Floating-Point Literal That Denotes No
            // Representable Value Is Malformed). A finite literal resolves to its exact `Decimal`.
            Leaf::Float(d) => {
                if d.is_finite_f64() {
                    Resolved::Float(d.clone())
                } else {
                    Resolved::Poison(Reject::coded(
                        Code::Malformed,
                        "float literal out of the Float64 range".to_string(),
                    ))
                }
            }
            // A bad-escape MARKER the reader emitted for a string literal with an UNRECOGNIZED escape
            // (`"\q"`). The reader cannot report through the artifact channel (its stderr is not the
            // diagnostic surface), so the COMPILER is where the lexical defect becomes a coded rejection:
            // CDZ0001 (`collections-and-text.md` §A String Literal's Escapes Are A Closed Set). Naming the
            // offending char makes the message actionable.
            Leaf::BadEscape(c) => Resolved::Poison(Reject::coded(
                Code::BadEscape,
                format!(
                    "unrecognized string escape `\\{c}` (the escape set is `\\n \\t \\r \\\\ \\\"`)"
                ),
            )),
            // A char literal (`#\a`) — a single Unicode scalar, a `Ty::Char` constant. Folds to
            // `Core::ConstChar` (equality/ordering by scalar value; `Char.to-int`/`from-int` later).
            Leaf::Char(c) => Resolved::Char(c),
            // A bad-char MARKER the reader emitted for a char literal naming a NON-scalar (`#\u+D800`, a
            // surrogate). Like `BadEscape`, the COMPILER turns the reader-detected lexical defect into a
            // coded rejection: CDZ0002 (`collections-and-text.md` §A Char Is A Single Unicode Scalar Value).
            Leaf::BadChar(s) => Resolved::Poison(Reject::coded(
                Code::BadChar,
                format!(
                    "`#\\{s}` does not name a Unicode scalar value (a code point in U+0000..=U+10FFFF, excluding the surrogates U+D800..=U+DFFF)"
                ),
            )),
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
                Some("list") => return resolve_list(db, id),
                Some("map") => return resolve_map(db, id),
                _ => {}
            }
            match db.ast.head_name(id) {
                Some("if") => resolve_if(db, id),
                Some(h @ ("and" | "or")) => resolve_connective(db, id, h == "and"),
                Some("not") => resolve_not(db, id),
                Some("match") => resolve_match(db, id),
                Some("quote") => resolve_quote(db, id),
                Some(h @ ("unquote" | "unquote-splicing")) => resolve_unquote(db, id, h),
                Some("quasiquote") => resolve_quasiquote(db, id),
                Some("bin") => resolve_bin(db, id),
                Some("|>") => resolve_pipeline(db, id),
                // The DESUGARED handle (`effects::desugar_handles` re-spells the canonical 5-child
                // source form to this internal head). A node still headed `handle` here is a leftover
                // the desugar did NOT rewrite — the old effect-name-less shape or a too-short handle —
                // which `resolve_handle` rejects with the canonical shape.
                Some(crate::effects::HANDLE_INTERNAL) => resolve_handle(db, id),
                Some("handle") => resolve_noncanonical_handle(db, id),
                Some("resume") => resolve_resume(db, id),
                Some("host") => resolve_host(db, id),
                Some("let") => resolve_let(db, id),
                Some("do") => resolve_do(db, id),
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
                        args: children[1..].into(),
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
/// The resolved form a top-level def at index `d` denotes: a nullary def denotes its body (`Ref`); a
/// def with parameters denotes a lambda `(params) body`; a bodyless (malformed) def is a rejection.
/// Shared by the flat and file-scoped step-2 paths of [`resolve_name`].
fn def_as_resolved(db: &Db, d: usize, name: &str) -> Resolved {
    let def = &db.defs[d];
    trace!(target: "rcdzc::resolve", %name, params = def.params.len(), "name → top-level def");
    match def.body {
        Some(body) if def.params.is_empty() => Resolved::Ref { value: body },
        Some(body) => Resolved::Lambda {
            params: def.params.clone().into(),
            body,
        },
        None => Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!("`{name}` has no body"),
        )),
    }
}

fn resolve_name(db: &Db, id: StructId, name: &str) -> Resolved {
    // 1. Lexical scope — nearest enclosing binder. A binder yields a `Ref` to its value occurrence
    // (a `let`/param/scalar-match binder) OR a `SumPayload` (a variant-pattern binder binds the sum's
    // payload, not a plain occurrence) — `binder_in` returns the full resolved form.
    //
    // Scope-FIRST is what makes binding lexical and shadowing well-defined: `lookup_scope` walks
    // parents to the NEAREST enclosing binder, so a name resolves to its closest binding, and a local
    // binding is found before (thus shadows) a top-level def or prelude entry of the same name.
    //= spec/capabilities/core-semantics.md#binding-is-lexical
    //# A name MUST resolve to the nearest enclosing binding of that name.
    //= spec/capabilities/core-semantics.md#shadowing-is-well-defined
    //# A binding that shadows an outer binding of the same name MUST take effect for references in its scope as defined by the corpus.
    if let Some(resolved) = lookup_scope(db, id, name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, "name → lexical scope");
        return resolved;
    }
    // 2. The module's own top-level definitions. A nullary def denotes its body; a def WITH parameters
    // denotes a lambda `(params) body` (so a call `(f a)` applies it by the ordinary application
    // path). This is what makes a top-level function callable by name — and, being resolved lazily
    // per reference, a forward/mutual reference resolves regardless of definition order.
    //
    // In a LINKED multi-file PACKAGE this step is FILE-SCOPED (`DESIGN-package-linking.md` §4): a bare
    // name resolves against the reference's OWN file's surface (its own defs + its imports), never a
    // sibling file's defs. A single-file compile carries no linkage, so `is_linked_package()` is false
    // and this falls straight through to the flat `def_by_name` below — byte-identical to before.
    if db.is_linked_package() {
        match db.file_scoped_def(id, name) {
            // The reference's file is known and `name` is visible there → that def.
            Some(Ok(d)) => {
                trace!(target: "rcdzc::resolve", node = id.0, %name, file_scoped = true, "name → file-scoped def");
                return def_as_resolved(db, d, name);
            }
            // The file is known but `name` is NOT visible there — do NOT leak a sibling's def. Fall
            // through to the type-decl + prelude steps (a package may still reference a prelude built-in
            // or a `(type …)`); if none matches it is unbound, correctly.
            Some(Err(())) => {}
            // The reference's file is INDETERMINATE — a synthesized / β-copied node (an inlined callee
            // body), which lies outside every file's demux range. β-copy hygiene: a name defined in
            // exactly ONE file across the whole package is unambiguous, so resolve it flat; a name
            // defined in MORE THAN ONE file cannot be attributed to a file here, so DECLINE rather than
            // guess the wrong sibling (decline-don't-miscompile). A name in NO file falls through to the
            // prelude/type steps.
            None => match db.package_def_name_count(name) {
                0 => {}
                1 => {
                    if let Some(d) = db.def_by_name(name) {
                        trace!(target: "rcdzc::resolve", node = id.0, %name, "name → package def (unambiguous, synthesized node)");
                        return def_as_resolved(db, d, name);
                    }
                }
                _ => {
                    trace!(target: "rcdzc::resolve", node = id.0, %name, "name AMBIGUOUS across files under a synthesized node — decline");
                    return Resolved::Poison(Reject::decline(format!(
                        "cross-file reference to `{name}` inside an inlined body is ambiguous \
                         (defined in more than one file); import it explicitly"
                    )));
                }
            },
        }
    } else if let Some(d) = db.def_by_name(name) {
        return def_as_resolved(db, d, name);
    }
    // 3. The module's own SUM declarations — `(type NAME …)` binds `NAME` to its synthesized record
    // (fields = variants), resolved EXACTLY like a top-level def (step 2): a lookup against the
    // occurrence-keyed `type_decls`, returning a `Ref` to the record. After defs (a def/local shadows a
    // type name) and before the prelude (a type name shadows a built-in). So `Option`, `Option.Some`,
    // and `(: x Option)` all take the ordinary member-access/`(meta t)` paths — no separate name map,
    // no sum special-case in resolve.
    if let Some(value) = db.type_decl_by_name(name) {
        // SAME-NAME NEWTYPE, in application/pattern-HEAD position: `(type UserId (UserId Int64))` binds
        // ONE name that must mean the CONSTRUCTOR when it heads an application `(UserId 42)` or a pattern
        // `(UserId n)`, and the TYPE everywhere else (`(: x UserId)`, a bare value). The type decl (above)
        // would always win by name — so when this atom HEADS its enclosing list (`child_ix == 0`, the same
        // position `head_ctor` dispatches `list`/`tuple` structurally) AND the name is a same-name newtype,
        // resolve to the variant CTOR instead. A non-head occurrence keeps the type. Position-based, not a
        // name special-case (`prelude-and-resolution.md` §Nothing Is Privileged By Name).
        // Only fire on a USER node — a real program `(Box 42)`. A SYNTHESIZED node (id ≥
        // `user_node_count`) heading a list is the sum's OWN ctor-result type expression `(Box a)` that
        // `sums::sum_applied` builds for a GENERIC same-name sum, where `Box` MUST stay the type record
        // (re-applied in type position via `(meta apply)` = `sum-ctor`). Firing there resolved `Box` to
        // the ctor and corrupted the arrow → the variant looked nullary. The value-vs-type-position
        // distinction the head-position rule alone can't see is supplied by the user/synth boundary.
        if db.child_ix_of(id) == 0
            && db.is_user_node(id)
            && let Some(ctor) = db.same_name_newtype_ctor(name)
        {
            trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = ctor.0, "name → same-name newtype ctor (head position)");
            return Resolved::Ref { value: ctor };
        }
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → sum type decl");
        return Resolved::Ref { value };
    }
    // 3b. The module's own EFFECT declarations — `(effect NAME …)` binds `NAME` to its synthesized record
    // (fields = operation values), resolved EXACTLY like a sum type decl: a lookup against the
    // occurrence-keyed `effect_decls`, returning a `Ref` to the record. So `E`, `E.op`, and a perform
    // `(E.op a)` all take the ordinary member-access/application paths — no separate name map, no effect
    // special-case in resolve (`prelude-and-resolution.md` §Nothing Is Privileged By Name).
    if let Some(value) = db.effect_decl_by_name(name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → effect decl");
        return Resolved::Ref { value };
    }
    // 3c. A BARE VARIANT CONSTRUCTOR of a user `(type …)` declaration — `NLit`/`NNil` for `(type Node
    // (NLit Int64) NNil)`, the same ctor field a qualified `(. Node NLit)` projects. A nullary variant
    // may be used bare as a VALUE (`NNil`) and a payload variant bare-applied (`(NLit 5)`); both bind to
    // the ctor field and take the ordinary member/application paths. This is the user-declaration analog
    // of the built-in sums binding bare `Some`/`None`/`Ok`/`Err` in the prelude map — after the type name
    // + effect decls (a type/effect name shadows a variant) and before the prelude (a variant shadows a
    // built-in), resolved generically off `type_decls` (no name special-case). FIRST-WINS across sums; a
    // qualified `(. Type Variant)` disambiguates a shared variant name.
    if let Some(value) = db.variant_ctor_by_name(name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → user sum variant ctor");
        return Resolved::Ref { value };
    }
    // 4. The prelude map — a built-in binds to its installed arena node (a record, for a module). The
    // same `Ref` a program binding produces, so member access / folding treats it identically.
    if let Some(&value) = db.prelude.get(name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → prelude");
        return Resolved::Ref { value };
    }
    // 3. Off the end of the lookup — the name is unbound. This is a REJECTION (the program is
    // ill-formed), not a decline: the unbound-name rule is unconditional. A reference that reaches here
    // has no enclosing binding (and is no top-level def / prelude entry), so it is a compile-time error:
    //= spec/capabilities/core-semantics.md#binding-is-lexical
    //# A reference to a name with no enclosing binding MUST be a compile-time error.
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
    // The "did you mean?" typo suggestion (the nearest in-scope name) is computed LAZILY, at the ONE site
    // that SURFACES an unbound name as a user fault (`infer::collect_node`) — NOT here. `resolved_of` is
    // consulted on many occurrences whose unbound Poison is NEVER surfaced (a pattern-binder name `x` in
    // `(V x)`, classified by `collect_pattern_binders`; a resolve done only to test a value's shape), and
    // the suggestion is an O(names-in-scope) candidate scan with a Levenshtein per candidate. Computing it
    // eagerly here made a match over an N-variant sum O(N²) (each of N arm binders resolved unbound, each
    // scanning all N variant names). Emitting the BARE unbound Poison here (the collector enriches it with
    // the nearest-name message + heuristic fix, `enrich_unbound`) keeps the diagnostic identical while the
    // scan runs at most once per SURFACED fault. See `infer::enrich_unbound`.
    Resolved::Poison(Reject::coded(Code::Unbound, format!("unbound name `{name}`")).at(id))
}

/// The nearest in-scope name to the unbound name at `id` — the "did you mean?" candidate a surfaced
/// unbound-name fault names. PUBLIC so `infer::collect_node` can compute it LAZILY at the fault-surfacing
/// site (the bare unbound Poison `resolve_name` emits carries only the node; the suggestion is attached
/// there so the O(scope) candidate scan runs at most once per surfaced fault, never per resolve). See
/// [`nearest_name_suggestion`].
pub(crate) fn nearest_unbound_suggestion(db: &Db, id: StructId, name: &str) -> Option<String> {
    nearest_name_suggestion(db, id, name)
}

/// The nearest in-scope name to an unbound `name` referenced at `id`, if one is close enough to be a
/// plausible typo — the candidate a "did you mean?" suggestion names. Enumerates every name a reference
/// at this point COULD have resolved to (the same four lookup tiers `resolve_name` walks: lexical
/// binders, this module's defs, its `(type …)`/`(effect …)`/variant declarations, and the prelude),
/// then picks the closest by edit distance under a length-relative threshold. `None` when the nearest
/// candidate is too far to be a typo (so a genuinely-unknown name gets the plain "unbound" message, not
/// a misleading suggestion).
///
/// Determinism (`spec/capabilities/diagnostics.md` §A Fix Is A Deterministic Function Of The Source):
/// candidates are considered in a fixed order and ties break on the lexicographically-smaller name, so
/// the suggestion is a pure function of the program — never dependent on hash-map iteration order.
fn nearest_name_suggestion(db: &Db, id: StructId, name: &str) -> Option<String> {
    // (A one-char name has no meaningful typo neighbour — that guard now lives in `suggest::nearest`, so
    // every suggestion site shares it, and this path need not repeat it.)
    // CONTEXT-AWARE candidate pool: the syntactic POSITION of the typo constrains which names could have
    // been meant, so the suggestion is one the fix would actually resolve (the one-shot rule), not merely
    // the lexically-nearest name of any kind. Three positions:
    //   • MEMBER OPERAND `(. name key)` — a `handle`'s effect (rewritten to `(. E op)` arms), `(. Int64
    //     max)`: only a member-accessible name (effect, type, prelude module) fits. A value/binder/variant
    //     ctor would move the fault (`(. Log op)` → "record has no field `op`") — the `handle Logg` case
    //     where variant `Log` was a worse pick than effect `Logr`.
    //   • TYPE EXPRESSION `(: expr name)` — a value or parameter annotation: only a TYPE name fits. A value
    //     def / binder / variant ctor would fail the annotation check ("requires a type, found a non-type")
    //     — the `(: p flg)` → `flag` (a value) case.
    //   • VALUE — everywhere else: every kind is a candidate (the original unrestricted pool).
    let parent = db.parent_of(id);
    let member_operand = matches!(parent, Some(p)
        if db.ast.as_form(p, ".").map(|t| t.first().copied()) == Some(Some(id)));
    // The type slot of a `(: expr TYPE)` form — TYPE is `tail[1]` (`as_form` drops the head).
    let type_expr = matches!(parent, Some(p)
        if db.ast.as_form(p, ":").and_then(|t| t.get(1).copied()) == Some(id));
    // Positions that admit only NON-VALUE names drop the value tiers (lexical binders, value defs) — a
    // member operand and a type expression are each such a position.
    let non_value_position = member_operand || type_expr;
    let mut candidates: Vec<String> = Vec::new();
    if !non_value_position {
        // Tier 1 — lexical scope: every binder visible where the reference sits (params, `let` bindings,
        // pattern binders). Only meaningful in value position — a type/member position takes no local.
        for (n, _occ) in visible_bindings(db, id) {
            candidates.push(n);
        }
        // Tier 2 — this module's top-level value definitions (value position only).
        for d in &db.defs {
            candidates.push(d.name.clone());
        }
        // Tier 2b — the boolean LITERALS `true`/`false` (value position only). They are lexer literals
        // (`Leaf::Bool`), not bound names, so they are NOT in scope/defs/prelude — yet a mis-cased
        // `True`/`False` (the cross-language habit) reads as an unbound NAME, and its one-shot fix is the
        // lowercase literal (which re-lexes as `Leaf::Bool`). Offering them as candidates makes `True` →
        // "did you mean `true`?" (edit distance 1, within the cutoff), the same did-you-mean an unbound
        // name gets — a literal is a valid replacement here exactly as a name is. (`TRUE` is distance 4,
        // beyond the cutoff, so only the common single-case-slip is suggested — no baseless guess.)
        candidates.push("true".to_string());
        candidates.push("false".to_string());
    }
    // Tier 3 — `(type …)` names (a type name fits BOTH a member operand `Int64.max` AND a type expr `(: x
    // Int64)`) + their variant CONSTRUCTORS (a value, so kept ONLY in value position) — and `(effect …)`
    // names (member-accessible, so a member-operand candidate; NOT a type, so excluded from a type expr).
    for t in &db.type_decls {
        candidates.push(t.name.clone());
        if !non_value_position {
            for v in &t.variants {
                candidates.push(v.name.clone());
            }
        }
    }
    if !type_expr {
        for e in &db.effect_decls {
            candidates.push(e.name.clone());
        }
    }
    // Tier 4 — the prelude's built-in names. In a NON-VALUE position drop the prelude's VARIANT
    // CONSTRUCTORS (`None`/`Some`/`Ok`/`Err`): a variant is a value, member-inaccessible AND not a type, so
    // suggesting `Nope`→`None` in `(. Nope op)` / `(: x Nope)` position would fail the one-shot rule. A
    // variant name is one some sum declares; collect that set and skip it. (A prelude MODULE/TYPE like
    // `Bytes`/`Int64` — the valid target in both non-value positions — is not a variant name, so it stays.)
    let variant_names: std::collections::HashSet<&str> = if non_value_position {
        db.type_decls
            .iter()
            .flat_map(|t| t.variants.iter().map(|v| v.name.as_str()))
            .collect()
    } else {
        std::collections::HashSet::new()
    };
    for key in db.prelude.keys() {
        if non_value_position && variant_names.contains(key.as_str()) {
            continue;
        }
        candidates.push(key.clone());
    }
    crate::diag::suggest::nearest(name, candidates)
}

/// Every lexical binding visible where node `id` sits, as `(name, binder-occurrence)` pairs,
/// innermost-first. A focused parent-walk covering the binder forms a typo'd reference is most likely to
/// have meant: `let` bindings, `fn`/`def` parameters, and a match arm's bare binder. It deliberately does
/// NOT model the rarer binder shapes (variant-payload, guard, handle-arm) — a reference near one of those
/// simply gets no lexical candidate and falls back to defs/prelude, or to the plain "unbound" message;
/// missing a rare candidate degrades gracefully, it never mis-suggests. (The exhaustive scope walk that
/// backs `cdz scope` lives in `sidecar::scope_at`; this stays in `resolve` to keep the layering — the
/// query API depends on resolve, not the reverse.)
fn visible_bindings(db: &Db, id: StructId) -> Vec<(String, StructId)> {
    let mut out: Vec<(String, StructId)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if !db.is_user_node(id) {
        return out;
    }
    let mut push = |n: &str, occ: StructId, out: &mut Vec<(String, StructId)>| {
        if seen.insert(n.to_string()) {
            out.push((n.to_string(), occ));
        }
    };
    let mut from = id;
    let mut cursor = db.parent_of(id);
    while let Some(form) = cursor {
        match db.ast.head_name(form) {
            // A `let` ascended from its BODY → every binding.
            Some("let") => {
                if let Some(tail) = db.ast.as_form(form, "let")
                    && let Some(bindings_occ) = tail.first().copied()
                    && Some(from) == tail.get(1).copied()
                    && let Struct::List(pairs) = db.ast.get(bindings_occ)
                {
                    for &pair in pairs {
                        if let Struct::List(kv) = db.ast.get(pair)
                            && kv.len() == 2
                            && let Some(n) = db.ast.as_name(kv[0])
                        {
                            push(n, kv[0], &mut out);
                        }
                    }
                }
            }
            // A `fn`/`def` ascended from its BODY → its parameters (the precomputed binder index).
            Some("fn") | Some("def") => {
                let body_occ = db
                    .ast
                    .as_form(form, "fn")
                    .or_else(|| db.ast.as_form(form, "def"))
                    .and_then(|t| t.get(1).copied());
                if Some(from) == body_occ {
                    // Sorted for determinism (the binder index is a hash map).
                    let mut params: Vec<(String, StructId)> = db
                        .scope_binders_of(form)
                        .map(|(n, occ)| (n.to_string(), occ))
                        .collect();
                    params.sort();
                    for (n, occ) in params {
                        push(&n, occ, &mut out);
                    }
                }
            }
            _ => {
                // A let's BINDINGS-LIST ascended from a pair → the bindings BEFORE it (sequential scope).
                // `form` IS the bindings-list; `let_of_bindings_list` confirms it (returns the enclosing
                // `let`, which we only need for the shape check).
                if let_of_bindings_list(db, form).is_some()
                    && let Struct::List(pairs) = db.ast.get(form)
                {
                    let end = db.child_ix_of(from).min(pairs.len());
                    for &pair in &pairs[..end] {
                        if let Struct::List(kv) = db.ast.get(pair)
                            && kv.len() == 2
                            && let Some(n) = db.ast.as_name(kv[0])
                        {
                            push(n, kv[0], &mut out);
                        }
                    }
                }
                // A match ARM `(pattern body)` ascended from `body`, pattern a bare binder → it binds.
                if let Struct::List(arm) = db.ast.get(form)
                    && arm.len() == 2
                    && Some(from) == arm.get(1).copied()
                    && let Some(n) = db.ast.as_name(arm[0])
                    && n != "_"
                    && db
                        .parent_of(form)
                        .and_then(|p| db.ast.as_form(p, "match"))
                        .is_some()
                {
                    push(n, arm[0], &mut out);
                }
            }
        }
        from = form;
        cursor = db.parent_of(form);
    }
    out
}

/// Walk parents from `id` to the nearest enclosing binding of `name`, returning the value occurrence
/// it is bound to. Nearest-wins gives shadowing for free (`core-semantics.md` §Binding Is Lexical,
/// §Shadowing Is Well-Defined). A `let` binds a name for the initializers that FOLLOW it and its body;
/// walking up from a reference, the first binder of `name` at or outside the reference's position is
/// the one in effect.
fn lookup_scope(db: &Db, id: StructId, name: &str) -> Option<Resolved> {
    // For a LOAD-TIME node, hop candidate-to-candidate via the precomputed scope-skip pointer, skipping
    // the non-binding spine (record/tuple/`.`/`if`/application forms `binder_in` would only reject).
    // Each hop lands on a form that MIGHT bind `name`, entered through the recorded child (the `from`
    // `binder_in` needs); a hop is O(1), so a reference costs O(enclosing BINDERS), not O(nesting
    // depth) — the deepdata/deeplet/bignest O(N²). The skip chain stays within load-time nodes (a
    // parent is always older than its child), so once we start on the skip path it remains covered.
    if db.scope_skip_covers(id) {
        let mut cursor = db.scope_skip_of(id);
        while let Some((form, from)) = cursor {
            if let Some(resolved) = binder_in(db, form, from, name) {
                return Some(resolved);
            }
            cursor = db.scope_skip_of(form);
        }
        return None;
    }
    // A node SYNTHESIZED after load (a β-reduced body copy) is not in the skip index — walk parents
    // exhaustively. Such a copy is shallow and self-contained, so the walk is cheap; and its ancestor
    // chain may cross back into covered load-time nodes, which the plain walk handles uniformly.
    lookup_scope_walk(db, id, name)
}

/// The exhaustive parent-by-parent scope walk — used for a node the skip index does not cover (a
/// synthesized post-load node). Identical semantics to the skip-driven path, visiting every enclosing
/// form; the skip index is purely an accelerator over this for the load-time arena.
fn lookup_scope_walk(db: &Db, id: StructId, name: &str) -> Option<Resolved> {
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
    // Read `form`'s head name ONCE and dispatch on it, rather than probing it with a cascade of
    // `as_form(form, "let"/"fn"/"def")` (each re-fetches the node, re-reads the head, and string-
    // compares). A lexical-scope walk ascends EVERY enclosing form — and in a deeply nested value
    // (`(record … (next (record … )))`, `(tuple a (tuple …))`) the vast majority are non-binding
    // forms (a record/tuple/`.`/`if`/application), so paying five string-compares each to conclude
    // "binds nothing" was the bulk of the O(depth)-per-reference walk (measured: `as_form` was ~45%
    // of self-time and O(N²) in call count on a depth-N nest). One `head_name` + `==` per form.
    let head = db.ast.head_name(form);
    match head {
        // Case 1: `form` is a `let`, ascended from its body → all bindings visible.
        Some("let") => {
            let tail = db.ast.as_form(form, "let")?;
            let bindings_occ = tail.first().copied()?;
            let body_occ = tail.get(1).copied();
            if Some(from) == body_occ {
                // The body sees EVERY binding — no `stop_before`. A bare-name binder resolves to a
                // `Ref`, a destructuring tuple-pattern binder to a `SumPayload` (handled inside).
                return last_binder_named(db, bindings_occ, name, None);
            }
            return None;
        }
        // Case 3: `form` is a `(fn (params) body)`, ascended from the body → its parameters bind. A
        // parameter reference resolves to the PARAMETER occurrence itself (a formal); the evaluator
        // substitutes the argument there at application, and `infer` gives an unsubstituted parameter a
        // fresh variable. The last param of `name` wins (shadowing among params, harmless).
        Some("fn") => {
            let tail = db.ast.as_form(form, "fn")?;
            let params_occ = tail.first().copied()?;
            let body_occ = tail.get(1).copied();
            if Some(from) == body_occ && matches!(db.ast.get(params_occ), Struct::List(_)) {
                // A parameter binds the name it declares — bare `a` or annotated `(: a T)` alike. The
                // last param of `name` wins. O(1) via the precomputed per-scope binder index (built once
                // at load; see `Db::binder_in_scope`) — a linear scan here was O(N²) when N body
                // references each ascended into an N-parameter signature.
                return db
                    .binder_in_scope(form, name)
                    .map(|v| Resolved::Ref { value: v });
            }
            return None;
        }
        // Case 4: `form` is a `(def (NAME param…) body)`, ascended from the body → the signature's
        // parameters (everything after NAME) bind. So a def-with-params body sees its parameters,
        // exactly as a lambda body sees its own. A parameter may be an annotated binder `(: a T)`.
        Some("def") => {
            let tail = db.ast.as_form(form, "def")?;
            let sig_occ = tail.first().copied()?;
            let body_occ = tail.get(1).copied();
            if Some(from) == body_occ && matches!(db.ast.get(sig_occ), Struct::List(_)) {
                // sig = [NAME, param…]; a reference binds to the matching PARAM's name occurrence,
                // last-wins. O(1) via the precomputed per-scope binder index (see `Db::binder_in_scope`).
                return db
                    .binder_in_scope(form, name)
                    .map(|v| Resolved::Ref { value: v });
            }
            return None;
        }
        _ => {}
    }
    // Case 2: `form` is a let's BINDINGS-LIST, ascended from pair `from` → the bindings BEFORE `from`
    // are visible (an initializer sees the earlier bindings, not itself or later ones). A bindings-list
    // has NO head name (it is a bare list of pairs), so this is only reached for a headless/other-headed
    // form — its own parent-shape check (`let_of_bindings_list`) confirms it. This is the let's
    // in-order scope: an initializer observes the bindings written before it and none written after,
    // and `last_binder_named` returns the LAST match, so a repeated name shadows the earlier one for the
    // initializers that follow.
    //= spec/capabilities/core-semantics.md#the-bindings-of-one-let-take-effect-in-order
    //# The bindings of a single `let` MUST take effect in the order they are written: each binding's initializer MUST observe the bindings written before it in the same `let`, and MUST NOT observe the bindings written after it.
    //= spec/capabilities/core-semantics.md#the-bindings-of-one-let-take-effect-in-order
    //# A binding whose name repeats an earlier binding in the same `let` MUST shadow the earlier one for the initializers and body that follow it, in accordance with §"Shadowing Is Well-Defined".
    if let_of_bindings_list(db, form).is_some() {
        return last_binder_named(db, form, name, Some(from));
    }
    // Case 5: `form` is a MATCH ARM `(pattern body)`, ascended from `body`, and `pattern` is a bare
    // BINDER name (not a literal, not `_`) equal to `name` → the binder binds the whole scrutinee for
    // this arm's body. The bound value IS the scrutinee, so a reference resolves to the scrutinee
    // occurrence — its type and (at lowering) its value flow straight through, no separate slot. Scoped
    // to THIS arm only: an arm is reached from the enclosing `(match …)` and this case fires ONLY when
    // ascending from that arm's own body, so a binder in one arm is invisible to another arm.
    //= spec/capabilities/core-semantics.md#bindings-introduced-by-a-pattern-are-scoped-to-its-branch
    //# A name a pattern binds MUST be in scope only in the branch guarded by that pattern.
    if let Some(scrutinee) = match_arm_binds(db, form, from, name) {
        return Some(Resolved::Ref { value: scrutinee });
    }
    // Case 5g: `form` is a GUARD `(guard <binder> <cond>)`, ascended from `<cond>` → the binder is in
    // scope in the guard (the guard `x < 0` reads the pattern's binder `x`). A guard reference ascends
    // into the guard form BEFORE reaching the arm, so it is caught here rather than at the arm (where
    // `from` would be the whole `(guard …)` pattern, not the cond). Binds the scrutinee, like Case 5.
    if let Some(scrutinee) = guard_cond_binds(db, form, from, name) {
        return Some(Resolved::Ref { value: scrutinee });
    }
    // Case 6: `form` is a MATCH ARM whose pattern binds `name` at a variant PAYLOAD, possibly NESTED —
    // `((. Sum V) binder)` (path `[Payload]`) or `(Some (Some binder))` (path `[Payload, Payload]`). The
    // binder binds the sub-value at that path (not the whole scrutinee, unlike Case 5). It resolves to a
    // `SumPayload` reading the path from the scrutinee; its type is the innermost variant's payload type.
    // Scoped to this arm.
    if let Some((scrutinee, steps, heads)) = match_arm_variant_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            steps: steps.into(),
            heads: heads.into(),
        });
    }
    // Case 6l: `form` is a MATCH ARM `((list a b …) body)` (ascended from `body`) whose FIXED-ARITY list
    // pattern binds `name` at element position `i`. Reading element `i` of the (list) scrutinee is the
    // SAME `Elem(i)` access a tuple-payload binder uses, so it reuses `SumPayload` with a bare `[Elem(i)]`
    // path (no `Payload` step, no head). Its type is the list's element type; a constant scrutinee folds
    // the element (`fold_sum_path`'s `ListNew` arm). Scoped to this arm. (A REST binder binds a SUBLIST,
    // not one element — a later increment; `list_pattern_element_binds` declines a pattern with `..`.)
    if let Some((scrutinee, index)) = list_pattern_element_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            steps: vec![crate::core::PathStep::Elem(index)].into(),
            heads: vec![].into(),
        });
    }
    // Case 6g: `form` is a GUARD `(guard <variant-pattern> <cond>)`, ascended from `<cond>` → a payload
    // binder of the variant pattern is in scope in the guard cond (`(guard (Some x) (> x 0))` — the guard
    // `(> x 0)` reads the payload binder `x`). Like Case 5g but the binder nests in a VARIANT pattern, so
    // it resolves to a `SumPayload` (not the whole scrutinee). A guard-cond reference ascends into the
    // `(guard …)` form BEFORE reaching the arm, so it is caught here (at the arm, `from` is the guard
    // wrapper, not the cond, so Case 6's guard branch would miss it).
    if let Some((scrutinee, steps, heads)) = guard_cond_variant_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            steps: steps.into(),
            heads: heads.into(),
        });
    }
    // Case 7: `form` is a HANDLE ARM `(op (params…) state body)`, ascended from `body`, and `name` is one
    // of the operation PARAMETERS or the STATE binder → it binds for this arm's body. Like a lambda
    // parameter, the binder resolves to its own occurrence (a `Param` formal) — the compile-time evaluator
    // substitutes the perform's arguments for the params and the current state for `state` when it
    // resolves the handler (E1c); until then a bare reference type-checks against a fresh variable. Scoped
    // to THIS arm (an arm is reached through the handle's arms-list, so one arm's binders are invisible to
    // another). State shadows a same-named param (last-wins), harmless in practice.
    if let Some(binder) = handle_arm_binds(db, form, from, name) {
        return Some(Resolved::Ref { value: binder });
    }
    // Case 8: `form` is a `(do f0 … fn)` SEQUENCING block, ascended from one of its forms `from` → a
    // do-local `(def …)` DECLARATION appearing BEFORE `from` binds `name` for `from`. Sequential,
    // backward-only scope like a `let` bindings-list: a form sees the declarations before it, last-wins (a
    // later `def` of the same name shadows an earlier one), and a declaration does NOT see itself or the
    // forms after it. The bound value is resolved by `do_def_binds` (a `Ref`/`Lambda`, exactly a top-level
    // def's shape) — so a later reference / call resolves and folds by the ordinary paths.
    //= spec/capabilities/core-semantics.md#a-declaration-in-a-sequencing-block-is-scoped-to-the-forms-that-follow-it
    //# A declaration form in a sequencing block MUST bind its name for the forms that follow it in that block, so that a name a declaration introduces is in scope without a separate binding form.
    if let Some(binder) = do_local_binds(db, form, from, name) {
        return Some(binder);
    }
    // Case R: `form` is a nested module's SYNTHESIZED RECORD — the join point every member body's parent
    // chain ascends through (a member's synth field lambda reuses the original body, so the body's
    // ancestors lead up to this record). A module's `(def …)` members are MUTUALLY visible in each other's
    // bodies (`core-semantics.md` §A Module Evaluates To A Record Of Its Exports: exported definitions are
    // in scope in each other's bodies, like top-level defs — NOT sequentially like a do-block), so resolve
    // `name` against ALL the module's members, including the one we ascended from (an exported function is
    // in scope in its OWN body → recursion). The bound value is `do_def_binds`'s `Ref`/`Lambda` over the
    // ORIGINAL member body, so a non-recursive call folds by β-reduction and a recursive one is caught by
    // `is_recursive` (then emitted as a runtime call, or declined if its callee is not a top-level def).
    if let Some(binder) = module_sibling_binds(db, form, name) {
        return Some(binder);
    }
    // Case B: `form` is a MATCH ARM `(pattern body)` whose pattern is a `(bin <seg>…)` binary pattern that
    // binds `name` at one of its segments — `(match b ((bin (u16 n)) n) …)`, the `n` in the body. The
    // binder decodes that segment from the scrutinee; it resolves to a `BinField` (the binary analogue of
    // Case 6's `SumPayload`). Scoped to this arm. An integer segment binder types `Ty::Int`, a bytes
    // segment binder `Ty::Bytes`; lowering decodes it from the scrutinee (const-fold / BN4 runtime read).
    if let Some((scrutinee, segs, seg_index)) = match_arm_bin_binds(db, form, from, name) {
        return Some(Resolved::BinField {
            scrutinee,
            segs: segs.into(),
            seg_index,
        });
    }
    // Case M: `form` is a MATCH ARM `((map (k v) … .. rest) body)`, ascended from `body`, whose map
    // pattern binds `name` — either a VALUE binder `v` at key `k` (a key-directed lookup) or the REST
    // binder (the scrutinee minus the named keys). Resolves to a `MapField` (the map analogue of Case 6's
    // `SumPayload` / Case B's `BinField`). Scoped to this arm.
    if let Some((scrutinee, key, named)) = match_arm_map_binds(db, form, from, name) {
        return Some(Resolved::MapField {
            scrutinee,
            key,
            named: named.into(),
        });
    }
    None
}

/// If `form` is a match ARM `(pattern body)` (ascended from its BODY) whose pattern is a `(map (k v) …
/// .. rest)` map pattern binding `name`, return `(scrutinee, key, named)`: the enclosing match's
/// scrutinee, `Some(key-occ)` when `name` is a VALUE binder at that key (else `None` for the REST
/// binder), and `named` the keys the pattern names (removed to form the rest map). `None` otherwise.
/// The map companion of [`match_arm_bin_binds`].
fn match_arm_map_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Option<StructId>, Vec<StructId>)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 || from != pb[1] {
        return None; // not an arm, or the reference is not from the arm's body
    }
    let (entries, rest) = map_pattern_of(db, pb[0])?;
    // `form`'s parent must be a `(match scrutinee arm…)` and `form` an arm (not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    let named: Vec<StructId> = entries.iter().map(|&(k, _)| k).collect();
    // A VALUE binder: `name` is the value position of some entry → carry that entry's KEY.
    for &(k, v) in &entries {
        if db.ast.as_name(v).is_some_and(|nm| nm == name && nm != "_") {
            return Some((scrutinee, Some(k), named));
        }
    }
    // The REST binder: `name` is the rest occurrence → the scrutinee minus the named keys.
    if rest.is_some_and(|r| db.ast.as_name(r).is_some_and(|nm| nm == name && nm != "_")) {
        return Some((scrutinee, None, named));
    }
    None
}

/// If `form` is a match ARM `(pattern body)` (ascended from its BODY) whose pattern is a `(bin <seg>…)`
/// binary pattern binding `name` at one of its segments, return `(scrutinee, segs, seg_index)` — the
/// enclosing match's scrutinee, the parsed segment list, and which segment's binder `name` is. `None`
/// otherwise. A segment's slot is a BINDER iff it is a bare name (not a literal — that is a match probe);
/// so `(bin (u32 0x89504E47) (bytes rest))` binds `rest` at index 1 but the literal at 0 is a probe. The
/// binary companion of [`match_arm_binds`]/[`match_arm_variant_binds`].
fn match_arm_bin_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<crate::resolved::Segment>, usize)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    let (pattern, body) = (pb[0], pb[1]);
    if from != body {
        return None;
    }
    // The pattern must be a `(bin …)` form. Re-parse its segment list via `resolve_bin` (pure over `&Db`,
    // so this is exactly the `Resolved::Bin` the pattern position resolves to).
    if db.ast.head_name(pattern) != Some("bin") {
        return None;
    }
    let Resolved::Bin { segs } = resolve_bin(db, pattern) else {
        return None;
    };
    // Which segment's slot is the bare name `name`? A literal slot is a probe, not a binder.
    let seg_index = segs.iter().position(|s| {
        db.ast
            .as_name(s.slot)
            .is_some_and(|nm| nm == name && nm != "_")
    })?;
    // `form`'s parent must be a `(match scrutinee arm…)` and `form` an arm (not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    Some((scrutinee, segs, seg_index))
}

/// If `form` is a `(do …)` block ascended from its child `from`, and a do-local `(def …)` before `from`
/// binds `name`, the value the name denotes (last-wins among the declarations before `from`). `None`
/// otherwise. Sequential backward scope: only declarations strictly before `from` are visible, matching a
/// `let` bindings-list (`last_binder_named` with `stop_before`).
fn do_local_binds(db: &Db, form: StructId, from: StructId, name: &str) -> Option<Resolved> {
    // The PROGRAM ROOT `(do …)` is the top-level block owned by the top-level scan (`db::scan_top_level`)
    // — its defs are resolved by the FILE-SCOPED `def_by_name` path (step 2 of `resolve_name`), which
    // enforces package linkage (a sibling file's def is invisible without an import). A lexical do-scope
    // binder here would bypass that (walking `app`'s `main` body up into the merged root would see
    // `lib`'s defs). So the root do binds NOTHING lexically; only a NESTED `(do …)` in expression
    // position is a do-scope. (A single-file program's root do is likewise covered by the scan.)
    if form == db.ast.root {
        return None;
    }
    let forms = db.ast.as_form(form, "do")?;
    // The window is the forms strictly BEFORE `from` (a form sees earlier declarations, not itself or
    // later ones). `from` is a direct child of the `do`, so its position among ALL children is the O(1)
    // `child_ix`; `forms` is the tail AFTER the `do` head at child index 0, so `from`'s index in `forms`
    // is `child_ix - 1` (a headed form, unlike a `let`'s headless bindings-list).
    let ix = db.child_ix_of(from);
    if ix == 0 {
        return None; // `from` is the `do` head itself, not a do-form (defensive)
    }
    let k = ix - 1;
    if forms.get(k) != Some(&from) {
        return None; // `from` is not a direct do-form (defensive; a genuine child always matches)
    }
    // Reverse-scan the earlier forms so the LAST declaration of `name` wins, stopping at the first hit.
    for &f in forms[..k].iter().rev() {
        if let Some(binder) = do_def_binds(db, f, name) {
            return Some(binder);
        }
        // A do-local `(module NAME …)` binds `NAME` to its synthesized record (fields = its exported defs,
        // built at load by `modules::synthesize`) — a `Ref` to the record, so `(. NAME field)` is ordinary
        // member access. The module analogue of a do-local `def` binding, resolved off the occurrence-keyed
        // `modules` index (no name special-case) — so the module's name is bound in its enclosing scope by
        // its own declaration, and a reference resolves to that record under the ordinary lexical rules.
        //= spec/capabilities/core-semantics.md#a-module-binds-its-name-in-its-enclosing-scope
        //# Evaluating a module MUST bind the module's declared name in the enclosing scope to the record of the module's exports, so that a module is named by its declaration without a separate binding form.
        //= spec/capabilities/core-semantics.md#a-module-binds-its-name-in-its-enclosing-scope
        //# A reference to a module's name in its enclosing scope MUST resolve to that export record under the same lexical scope and shadowing rules as any other binding.
        if let Some(tail) = db.ast.as_form(f, "module")
            && tail.first().and_then(|&n| db.ast.as_name(n)) == Some(name)
            && let Some(record) = db.module_synth_by_occ(f)
        {
            return Some(Resolved::Ref { value: record });
        }
    }
    // No SEQUENTIAL (backward-visible) declaration bound `name`. A do-local FUNCTION declaration, though,
    // is in scope in its OWN body (self-recursion) and in a SIBLING function's body regardless of order
    // (mutual recursion) — a function group in a `do` is mutually visible exactly like a module's members
    // or the top-level defs, not strictly sequential like a value binding. So scan EVERY form (including
    // `from` itself and the ones after it) for a FUNCTION def of `name` — accepting ONLY a `Lambda` (a
    // `(def (f p…) BODY)` with parameters), never a `Ref`. This keeps a VALUE def strictly backward
    // (`(do (def x 5) (def x (+ x 10)) x)` = 15 — the second `x` sees only the first, not itself), while a
    // recursive/forward FUNCTION reference resolves. First match wins across the block (a duplicate
    // function name is a separate well-formedness concern).
    forms
        .iter()
        .filter_map(|&f| match do_def_binds(db, f, name) {
            lam @ Some(Resolved::Lambda { .. }) => lam,
            _ => None,
        })
        .next()
}

/// If `form` is a nested module's SYNTHESIZED RECORD, resolve `name` against the module's `(def …)`
/// members — the value the sibling denotes (`do_def_binds`'s `Ref`/`Lambda` over the member's ORIGINAL
/// body). `None` if `form` is not a module synth record or no member is named `name`. A module's members
/// are MUTUALLY visible (unlike a do-block's sequential scope), so this scans ALL members with no
/// stop-before — including the one the reference sits inside, which is what lets an exported function
/// call itself (recursion) or a forward sibling.
fn module_sibling_binds(db: &Db, form: StructId, name: &str) -> Option<Resolved> {
    // Recognize `form` as a module synth record and recover its `(module …)` declaration occurrence (the
    // reverse of `modules::synthesize`); its members are the module form's tail after NAME.
    let module_form = db.module_by_synth_record(form)?;
    let members = db.ast.as_form(module_form, "module")?.get(1..)?;
    // FIRST-wins across the members (a duplicate name is a separate concern; the sum/def indices resolve a
    // shared name first-wins too). A NESTED `(module inner …)` member binds `inner` to its synthesized
    // record — the same `Ref` the `(. outer inner)` projection folds to — so a sibling def's body may
    // reference the inner module by bare name, exactly as it references a sibling def. `do_def_binds`
    // yields exactly the `Ref`/`Lambda` a top-level/do-local def of the same shape would — so the ordinary
    // application/fold paths apply uniformly.
    members.iter().find_map(|&m| {
        if db
            .ast
            .as_form(m, "module")
            .and_then(|t| t.first())
            .and_then(|&n| db.ast.as_name(n))
            == Some(name)
            && let Some(record) = db.module_synth_by_occ(m)
        {
            return Some(Resolved::Ref { value: record });
        }
        do_def_binds(db, m, name)
    })
}

/// If `form` is a handle arm `(op (params…) state body)` ascended from its `body`, and `name` matches
/// the state binder or one of the operation parameters, the binder's NAME occurrence — the value the
/// name binds (an ordinary formal, resolved to a `Param` at that occurrence via `is_param_occurrence`).
/// `None` otherwise. The state binder shadows a same-named parameter (checked first, last-wins).
fn handle_arm_binds(db: &Db, form: StructId, from: StructId, name: &str) -> Option<StructId> {
    // `form` must be a 4-element arm whose 4th element (the body) is `from`, and it must actually be a
    // handle arm (parent shape), not any incidental 4-element list.
    let Struct::List(parts) = db.ast.get(form) else {
        return None;
    };
    if parts.len() != 4 || parts[3] != from || !is_handle_arm(db, form) {
        return None;
    }
    // The STATE binder (element 2) — a bare name binding the current fold state. Shadows a param.
    if db.ast.as_name(parts[2]) == Some(name) && name != "_" {
        return Some(parts[2]);
    }
    // The operation PARAMETERS (element 1) — a `(params…)` list, or `()` for a nullary operation. Last
    // match wins (shadowing among params is harmless).
    if let Struct::List(ps) = db.ast.get(parts[1]) {
        for &p in ps.iter().rev() {
            if db.ast.as_name(p) == Some(name) && name != "_" {
                return Some(p);
            }
        }
    }
    None
}

/// Whether `arm` is a HANDLE ARM — its parent is a handle's arms-list (the handle's 2nd tail element).
/// Used by `handle_arm_binds` and `is_binding_candidate` to confirm an arm's shape from a node up.
pub(crate) fn is_handle_arm(db: &Db, arm: StructId) -> bool {
    let Some(arms_list) = db.parent_of(arm) else {
        return false;
    };
    let Some(handle) = db.parent_of(arms_list) else {
        return false;
    };
    // The (desugared) handle is `(handle-internal INIT ARMS BODY)`; ARMS is its 2nd tail element.
    db.ast
        .as_form(handle, crate::effects::HANDLE_INTERNAL)
        .and_then(|t| t.get(1).copied())
        == Some(arms_list)
}

/// If `form` is a guard `(guard <binder> <cond>)` ascended from its `<cond>`, and `<binder>` is a bare
/// binder name equal to `name`, and the guard is the pattern of an enclosing match arm, the match's
/// SCRUTINEE occurrence — the value the binder binds (a guard reads the pattern's binder). `None`
/// otherwise. Complements `match_arm_binds`: a reference in the guard cond ascends into the `(guard …)`
/// form (this case) before it would reach the arm.
fn guard_cond_binds(db: &Db, form: StructId, from: StructId, name: &str) -> Option<StructId> {
    // `form` must be `(guard <binder> <cond>)`, ascended from the cond (the third element).
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    // The binder must be a bare name matching `name` (not the wildcard `_`).
    let pat_name = db.ast.as_name(g[0])?;
    if pat_name != name || pat_name == "_" {
        return None;
    }
    // The guard must be the PATTERN of a match arm `((guard …) body)` whose parent is a `(match …)`.
    let arm = db.parent_of(form)?;
    let Struct::List(pb) = db.ast.get(arm) else {
        return None;
    };
    if pb.len() != 2 || pb[0] != form {
        return None; // `form` must be the arm's pattern position
    }
    let matchf = db.parent_of(arm)?;
    let mtail = db.ast.as_form(matchf, "match")?;
    let scrutinee = *mtail.first()?;
    if arm == scrutinee {
        return None;
    }
    Some(scrutinee)
}

/// If `form` is a guard `(guard <variant-pattern> <cond>)` ascended from its `<cond>`, and the variant
/// pattern binds `name` at a payload (possibly nested), the `(scrutinee, path, heads)` for a `SumPayload`
/// read — the payload-binder analogue of [`guard_cond_binds`]. `(guard (Some x) (> x 0))` binds `x` at
/// `[Payload]` for the guard cond. `None` otherwise. Complements [`match_arm_variant_binds`]: a reference
/// in the guard cond ascends into the `(guard …)` form (this case) before it would reach the arm (where
/// `from` is the guard wrapper, not the cond, so that case's guard-cond branch cannot fire).
fn guard_cond_variant_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<crate::core::PathStep>, Vec<StructId>)> {
    // `form` must be `(guard <variant-pattern> <cond>)`, ascended from the cond.
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    let pattern = g[0];
    // The guard must be the PATTERN of a match arm `((guard …) body)` whose parent is a `(match …)`.
    let arm = db.parent_of(form)?;
    let Struct::List(pb) = db.ast.get(arm) else {
        return None;
    };
    if pb.len() != 2 || pb[0] != form {
        return None; // `form` must be the arm's pattern position
    }
    let matchf = db.parent_of(arm)?;
    let mtail = db.ast.as_form(matchf, "match")?;
    let scrutinee = *mtail.first()?;
    if arm == scrutinee {
        return None;
    }
    // Descend the variant pattern to find where `name` is bound (its payload path + per-step heads).
    let mut path = Vec::new();
    let mut heads = Vec::new();
    if find_binder_in_pattern(db, pattern, name, &mut path, &mut heads) {
        Some((scrutinee, path, heads))
    } else {
        None
    }
}

/// If `form` is a match ARM `(pattern body)` whose parent is a `(match scrutinee arm…)`, ascended from
/// the arm's `body` (or, for a GUARDED arm, its guard cond), and `pattern` is a bare BINDER name equal to
/// `name` (not a literal, not the wildcard `_`), the enclosing match's SCRUTINEE occurrence — the value
/// the binder binds. `None` otherwise. A binder pattern binds the whole scrutinee for its arm's body AND
/// its guard (the guard `x < 0` in `x if x < 0` reads the binder), so a reference in either resolves
/// straight to the scrutinee. The pattern may itself be a guarded pattern `(guard <binder> <cond>)` — the
/// binder is `<binder>`; the guard cond `<cond>` is where a guard reference is ascended from.
fn match_arm_binds(db: &Db, form: StructId, from: StructId, name: &str) -> Option<StructId> {
    // `form` must be a 2-element `(pattern body)` list.
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    let (pattern, body) = (pb[0], pb[1]);
    // The BINDER-carrying pattern: a bare name directly, or `(guard <binder> <cond>)` where `<binder>` is
    // the pattern. We ascended into this arm from either the BODY or (for a guarded arm) the guard COND;
    // in both positions the binder is in scope.
    let (binder_pat, guard_cond) = match db.ast.as_form(pattern, "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pattern, None),
    };
    // `from` must be the body, or the guard cond (a reference under the guard binds the same scrutinee).
    if from != body && Some(from) != guard_cond {
        return None;
    }
    // The binder must be a bare name — matching `name`, NOT a literal or the wildcard `_`.
    let pat_name = db.ast.as_name(binder_pat)?;
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

/// If `form` is a match arm `((list a b …) body)` ascended from `body`, and the list pattern binds `name`
/// at LEADING element position `i`, return `(scrutinee, i)`. Handles a fixed-arity `(list a b)` AND the
/// LEADING binders of a rest pattern `(list a b .. rest)` (the binders BEFORE `..`) — both read a definite
/// element index via `SumPayload{Elem(i)}`. The REST binder itself (the name after `..`) binds a SUBLIST,
/// not one element, so it is NOT matched here (it resolves inert; a used rest sublist is a later
/// increment). `None` if not a `(list …)` arm binding `name` at a leading position.
fn list_pattern_element_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, usize)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 || pb[1] != from {
        return None; // must be `(pattern body)` ascended from the body
    }
    let elems = db
        .ast
        .as_ctor_form(pb[0], "list")
        .or_else(|| db.ast.as_form(pb[0], "list"))?;
    // The LEADING positions are those before a `..` marker (all of them for a fixed-arity pattern).
    let lead = elems
        .iter()
        .position(|&e| db.ast.as_name(e) == Some(".."))
        .unwrap_or(elems.len());
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    // The FIRST leading element position bound to `name` (a bare name, not `_`).
    elems[..lead]
        .iter()
        .position(|&e| db.ast.as_name(e) == Some(name) && name != "_")
        .map(|i| (scrutinee, i))
}

/// Whether `id` is a NAME occurrence that is a `(list …)` match-pattern binder in the PATTERN position — a
/// LEADING element binder, the `..` marker, OR the rest binder — all of which must resolve INERT (not a
/// looked-up value) so walking the arm's pattern never reports them unbound. (A body reference to a
/// leading binder resolves to `SumPayload` via Case 6l; the rest binder is inert until a used-sublist
/// increment.) Mirrors the arm/scrutinee shape `list_pattern_element_binds` requires.
fn is_list_pattern_element_occurrence(db: &Db, id: StructId) -> bool {
    let Some(list) = db.parent_of(id) else {
        return false;
    };
    let Some(elems) = db.ast.as_form(list, "list") else {
        return false;
    };
    if !elems.contains(&id) {
        return false; // not an element of the list
    }
    let Some(arm) = db.parent_of(list) else {
        return false;
    };
    let Struct::List(pb) = db.ast.get(arm) else {
        return false;
    };
    if pb.len() != 2 || pb[0] != list {
        return false; // the list must be the arm's PATTERN (first element)
    }
    let Some(matchf) = db.parent_of(arm) else {
        return false;
    };
    match db.ast.as_form(matchf, "match") {
        Some(mtail) => mtail.first().copied() != Some(arm) && mtail.contains(&arm),
        None => false,
    }
}

/// The MAP PATTERN `(map (k v) … .. rest)` of the arm whose pattern is `pat`, as `(entries, rest)`:
/// each entry a `(key-occ, value-binder-occ)` pair, `rest` the optional rest-binder occurrence (after
/// `..`). `None` if `pat` is not a `(map …)` form or is malformed. Shared by the binder-occurrence
/// classifier, Case M, and `lower_match_map` — the one place a map PATTERN's shape is read.
/// A parsed map PATTERN: the `(key-occ, value-binder-occ)` entry pairs, plus the optional rest-binder
/// occurrence (after `..`).
pub(crate) type MapPattern = (Vec<(StructId, StructId)>, Option<StructId>);

pub(crate) fn map_pattern_of(db: &Db, pat: StructId) -> Option<MapPattern> {
    // A map pattern is a `(map …)` form — either the STRING-head primitive `("map" …)` or the NAME-head
    // `(map …)` (the shadowable alias, how the corpus writes it) — its tail is entry pairs, optionally
    // ending in `.. rest`. Accept both spellings (like the list matcher's `as_ctor_form.or_else(as_form)`).
    let tail = db
        .ast
        .as_ctor_form(pat, "map")
        .or_else(|| db.ast.as_form(pat, "map"))?;
    // Split at a `..` marker: the entries before it, then exactly one rest binder after.
    let (entries_tail, rest) = match tail.iter().position(|&e| db.ast.as_name(e) == Some("..")) {
        Some(i) => {
            if i + 2 != tail.len() {
                return None; // `..` must be followed by exactly one rest binder
            }
            (&tail[..i], Some(tail[i + 1]))
        }
        None => (tail, None),
    };
    let mut entries = Vec::with_capacity(entries_tail.len());
    for &entry in entries_tail {
        match db.ast.get(entry) {
            Struct::List(items) if items.len() == 2 => entries.push((items[0], items[1])),
            _ => return None,
        }
    }
    Some((entries, rest))
}

/// Whether `id` is a BINDER occurrence of a map PATTERN — a VALUE binder (the `v` in a `(k v)` entry) or
/// the REST binder (after `..`) of an arm's `(map …)` pattern. Such an occurrence names a binding, not a
/// value, so it resolves INERT (a body reference resolves via Case M `map_pattern_binds`). The KEY
/// position is NOT a binder — it is an ordinary value expression, so it is excluded here (resolves
/// normally). Mirrors `is_list_pattern_element_occurrence`.
fn is_map_pattern_binder_occurrence(db: &Db, id: StructId) -> bool {
    // Ascend: a value binder's parent is the entry pair whose parent is the `(map …)` pattern; the rest
    // binder's parent is the `(map …)` pattern directly. In both cases find the enclosing `(map …)` that
    // is an arm's PATTERN.
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    // The candidate map-pattern node: either `parent` (rest binder, direct child of the map) or
    // `grandparent` (value binder, inside a `(k v)` entry).
    let map_pat_candidates = [Some(parent), db.parent_of(parent)];
    for cand in map_pat_candidates.into_iter().flatten() {
        let Some((entries, rest)) = map_pattern_of(db, cand) else {
            continue;
        };
        // Is `cand` an arm's PATTERN (first element of a 2-element arm under a `(match …)`)?
        let Some(arm) = db.parent_of(cand) else {
            continue;
        };
        let Struct::List(pb) = db.ast.get(arm) else {
            continue;
        };
        if pb.len() != 2 || pb[0] != cand {
            continue;
        }
        let Some(matchf) = db.parent_of(arm) else {
            continue;
        };
        let is_arm = matches!(db.ast.as_form(matchf, "match"),
            Some(mtail) if mtail.first().copied() != Some(arm) && mtail.contains(&arm));
        if !is_arm {
            continue;
        }
        // `id` is a binder iff it is a VALUE position of some entry, or the REST binder. (A KEY position
        // is NOT — it resolves as a value.)
        if rest == Some(id) || entries.iter().any(|&(_, v)| v == id) {
            return true;
        }
    }
    false
}

/// If `form` is a match ARM `(pattern body)` (ascended from its BODY) whose pattern binds `name` at a
/// variant PAYLOAD — possibly NESTED — return `(scrutinee, path, innermost_variant_head)`. The scrutinee
/// is the enclosing match's; the path is the `Payload`/`Elem` steps from the scrutinee down to the
/// binder; the innermost head is the variant constructor directly enclosing the binder (its `(-> payload
/// Sum)` gives the binder's type). `None` if the arm's pattern does not bind `name` at a variant
/// payload. Handles `(Some x)` (path `[Payload]`) and `(Some (Some y))` (path `[Payload, Payload]`); a
/// tuple-payload destructure (`(NPrim (tuple h a b))`) is a later increment (the descent stops at it).
fn match_arm_variant_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<crate::core::PathStep>, Vec<StructId>)> {
    // `form` = `(pattern body)`. A variant payload binder from `(Some x)` is in scope in the arm's BODY
    // and — for a GUARDED arm `((guard (Some x) <cond>) body)` — in the guard COND too (the guard
    // `(> x 0)` reads the payload binder). So the PATTERN we descend is `pattern`, EXCEPT when it is a
    // `(guard <inner-pattern> <cond>)` wrapper: then the binder-carrying pattern is `<inner-pattern>` and
    // a reference is accepted from either the body OR the guard cond. (Case 5g handles the WHOLE-scrutinee
    // bare-binder guard; this handles a guard over a VARIANT pattern, whose payload binder nests.)
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    let body = pb[1];
    let (pattern, guard_cond) = match db.ast.as_form(pb[0], "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pb[0], None),
    };
    // Accept a reference ascended from the body, or (for a guarded arm) from the guard cond.
    if from != body && Some(from) != guard_cond {
        return None;
    }
    // `form`'s parent must be a `(match scrutinee arm…)`, with `form` an arm (not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    // Descend the pattern to find where `name` is bound, accumulating the access path + per-step heads. A
    // TOP-LEVEL TUPLE pattern `(tuple x y)` (matching directly on a tuple scrutinee, not inside a variant
    // payload) descends via `find_binder_in_tuple` — its elements bind at `Elem(i)` from the scrutinee
    // root, no `Payload` step. A variant pattern descends via `find_binder_in_pattern` as before. (A
    // top-level RECORD pattern is a later increment; a tuple scrutinee is the common structural-match
    // shape — `(match (tuple a b) ((tuple x y) …))`.)
    // A `(bin …)` binary pattern is NOT a variant pattern — its binders bind DECODED segments, resolved
    // by Case B (`match_arm_bin_binds` → `BinField`), not a `SumPayload` path. Excluded here so a segment
    // binder is not mis-descended as a variant/tuple payload (which would give it a spurious `Elem` path).
    if db.ast.head_name(pattern) == Some("bin") {
        return None;
    }
    let mut path = Vec::new();
    let mut heads = Vec::new();
    let found = if is_tuple_pattern(db, pattern) {
        find_binder_in_tuple(db, pattern, name, &mut path, &mut heads)
    } else {
        find_binder_in_pattern(db, pattern, name, &mut path, &mut heads)
    };
    if found {
        Some((scrutinee, path, heads))
    } else {
        None
    }
}

/// Descend a variant PATTERN looking for the payload binder `name`, appending its access-path steps to
/// `path` and the variant head at each `Payload` step to `heads`. Returns `true` if found. A variant
/// pattern is `(head arg…)` where `head` is a member `(. Sum V)` or a bare variant name; its single
/// payload arg is either the bare binder (found — one `Payload` step, this head), a NESTED variant
/// pattern (recurse, adding a `Payload` step + this head), or a nested TUPLE pattern `(tuple p0 p1…)`
/// (the payload is a tuple — a `Payload` step to reach the tuple, then descend each element by `Elem(i)`
/// via `find_binder_in_tuple`). `heads` carries one entry per `Payload` step only (an `Elem` step needs
/// no head — its type is read by tuple-indexing in `infer`).
fn find_binder_in_pattern(
    db: &Db,
    pattern: StructId,
    name: &str,
    path: &mut Vec<crate::core::PathStep>,
    heads: &mut Vec<StructId>,
) -> bool {
    let Struct::List(app) = db.ast.get(pattern) else {
        return false;
    };
    if app.len() < 2 {
        return false; // a nullary variant pattern binds nothing; a lone head has no payload.
    }
    let head = app[0];
    // The head is the variant CONSTRUCTOR — a `(. Sum V)` member OR a bare variant NAME. But the
    // compound-VALUE constructor names (`list`/`tuple`/`record`/`map`) are NOT variant heads: a `(list a
    // b)` / `(tuple a b)` pattern is a COLLECTION/tuple destructure, handled by its own binder case — so
    // exclude them here, or this would mis-bind `(list a b)`'s elements at a `[Payload]` variant path.
    let is_compound_ctor = db
        .ast
        .as_name(head)
        .is_some_and(|h| matches!(h, "list" | "tuple" | "record" | "map"));
    let head_ok = !is_compound_ctor
        && (db.ast.as_form(head, ".").is_some() || db.ast.as_name(head).is_some());
    if !head_ok {
        return false;
    }
    // A MULTI-PAYLOAD variant pattern `(Cons h t)` — more than one payload arg — is sugar for the
    // single-tuple-payload form `(Cons (tuple h t))`: the runtime boxes the payloads as ONE tuple handle
    // (see `lower::pattern_constraints` / the `SumNew` backend), so each arg destructures a tuple ELEMENT.
    // One `Payload` step reaches the payload tuple, then each arg descends at `Elem(i)` exactly as an
    // explicit tuple pattern's elements do. (`find_binder_in_tuple` records/undoes the `Elem(i)` step per
    // position, so `path`/`heads` reflect only the found binder's path.)
    if app.len() > 2 {
        path.push(crate::core::PathStep::Payload);
        heads.push(head);
        let path_len = path.len();
        let heads_len = heads.len();
        for (i, &arg) in app[1..].iter().enumerate() {
            path.push(crate::core::PathStep::Elem(i));
            let found = if let Some(arg_name) = db.ast.as_name(arg) {
                arg_name == name && arg_name != "_"
            } else if is_tuple_pattern(db, arg) {
                find_binder_in_tuple(db, arg, name, path, heads)
            } else {
                find_binder_in_pattern(db, arg, name, path, heads)
            };
            if found {
                return true;
            }
            path.truncate(path_len);
            heads.truncate(heads_len);
        }
        // Not found in any payload position — undo the `Payload` step this pattern pushed.
        path.pop();
        heads.pop();
        return false;
    }
    let arg = app[1];
    // The payload arg is the bare binder `name` (found here), a nested TUPLE pattern (the payload is a
    // tuple — descend its elements), or a NESTED variant pattern (recurse) — each adds one `Payload`
    // step + this `head` to reach the payload, then continues into the arg.
    if let Some(arg_name) = db.ast.as_name(arg) {
        if arg_name == name && arg_name != "_" {
            path.push(crate::core::PathStep::Payload);
            heads.push(head);
            return true;
        }
        return false; // a different bare binder / wildcard — not `name`
    }
    path.push(crate::core::PathStep::Payload);
    heads.push(head);
    if is_tuple_pattern(db, arg) {
        return find_binder_in_tuple(db, arg, name, path, heads);
    }
    find_binder_in_pattern(db, arg, name, path, heads)
}

/// Whether `id` is a tuple PATTERN `(tuple p0 p1…)` — a `tuple` NAME head (the shadowable alias the
/// reader keeps in a pattern) or the `"tuple"` string-literal primitive. Used to route a variant's
/// tuple payload into element-by-element descent.
fn is_tuple_pattern(db: &Db, id: StructId) -> bool {
    db.ast.as_form(id, "tuple").is_some() || db.ast.head_ctor(id) == Some("tuple")
}

/// Descend a TUPLE pattern `(tuple p0 p1…)` looking for the binder `name` in one of its element
/// positions, appending an `Elem(i)` step for the element that (transitively) binds `name`. An element
/// may itself be a bare binder (found — one `Elem(i)` step), a nested variant pattern (recurse via
/// `find_binder_in_pattern`), or a nested tuple pattern (recurse here). No head is pushed for an `Elem`
/// step (its type comes from tuple-indexing, not a variant head).
fn find_binder_in_tuple(
    db: &Db,
    pattern: StructId,
    name: &str,
    path: &mut Vec<crate::core::PathStep>,
    heads: &mut Vec<StructId>,
) -> bool {
    let elems: &[StructId] = db
        .ast
        .as_form(pattern, "tuple")
        .or_else(|| db.ast.as_ctor_form(pattern, "tuple"))
        .unwrap_or(&[]);
    for (i, &elem) in elems.iter().enumerate() {
        // Try this element position. Record the `Elem(i)` step, then match the element pattern; on a
        // miss, undo the step and try the next position (so `path`/`heads` reflect only the found path).
        let path_len = path.len();
        let heads_len = heads.len();
        path.push(crate::core::PathStep::Elem(i));
        let found = if let Some(elem_name) = db.ast.as_name(elem) {
            elem_name == name && elem_name != "_"
        } else if is_tuple_pattern(db, elem) {
            find_binder_in_tuple(db, elem, name, path, heads)
        } else {
            find_binder_in_pattern(db, elem, name, path, heads)
        };
        if found {
            return true;
        }
        path.truncate(path_len);
        heads.truncate(heads_len);
    }
    false
}

// (The parameter-name extraction that `binder_in`'s Case-3/Case-4 used lives in `db::build_scope_binders`
// now: the per-scope binder index is precomputed once at load, so resolution probes it in O(1) rather
// than re-deriving each parameter's name on every reference. `param_annot_ty` reads a binder's `(: a T)`
// annotation directly where it needs the type.)

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
) -> Option<Resolved> {
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
    // FAST PATH: a bindings-list of all bare-name bindings is indexed by name (last-wins positions), so
    // the answer is O(log N) — a `partition_point` for the last binding before `end`. This is byte-
    // identical to the reverse scan below for such a list (both return the last bare `Ref { value }` in
    // the window), and turns a wide accumulation `let` from O(N²) into O(N log N). A list with a
    // DESTRUCTURING binding is absent from the index (`None`), so it falls through to the linear walk,
    // which alone handles the `SumPayload` pattern-binder cases.
    if let Some(hit) = db.let_binder_before(bindings_occ, name, end) {
        return hit.map(|value| Resolved::Ref { value });
    }
    // Scan the in-scope pairs in REVERSE and return the first match — the last binder wins, and a
    // reverse walk lets us stop as soon as we find it rather than scanning the whole prefix.
    for &pair in pairs[..end].iter().rev() {
        if let Struct::List(kv) = db.ast.get(pair)
            && kv.len() == 2
        {
            // An ANNOTATED binding `((: pat T) V)` — peel the annotation to the inner pattern `pat`; the
            // binder is `pat`'s, and the type `T` is a constraint on `V` (checked when the let is lowered,
            // CDZ0203 on contradiction). So `(: x Int64)` binds `x` and `(: (tuple a b) T)` destructures,
            // both exactly as the un-annotated form does — the annotation does not change WHAT `name`
            // binds, only constrains its type.
            let lhs = match db.ast.as_form(kv[0], ":") {
                Some(ann) if ann.len() == 2 => ann[0],
                _ => kv[0],
            };
            // A bare-name binding `(x V)` binds `name` directly to the value occurrence `V` — a `Ref`,
            // the common case (an allocation-free early return, the hot path).
            if db.ast.as_name(lhs) == Some(name) {
                return Some(Resolved::Ref { value: kv[1] });
            }
            // A DESTRUCTURING binding `((tuple a b) V)` — the LHS is a tuple PATTERN. A reference to one
            // of its element binders resolves to a `SumPayload` reading that element from the value `V`
            // (path `[Elem(i)]`, possibly nested), EXACTLY as a `(match V ((tuple a b) …))` arm binder
            // does (`match_arm_variant_binds` Case 6). The binding position IS a single-arm irrefutable
            // match; reusing `find_binder_in_tuple` gives the desugar with zero new IR. (Refutability +
            // linearity are enforced when the `let` is lowered, so an ill-formed binding still faults —
            // this lookup only routes an in-scope binder to its value.)
            if is_tuple_pattern(db, lhs) {
                let mut path = Vec::new();
                let mut heads = Vec::new();
                if find_binder_in_tuple(db, lhs, name, &mut path, &mut heads) {
                    return Some(Resolved::SumPayload {
                        scrutinee: kv[1],
                        steps: path.into(),
                        heads: heads.into(),
                    });
                }
            }
            // A CONSTRUCTOR destructuring binding `((Id.Mk n) V)` — the LHS is a single-variant-sum
            // pattern (irrefutable: its sole constructor always matches; refutability is enforced at
            // lowering by `check_binding_pattern`). A reference to one of its payload binders resolves to a
            // `SumPayload` reading that payload from `V` — path `[Payload, …]` with the ctor `head`(s),
            // EXACTLY as a `(match V ((Id.Mk n) …))` arm binder does (`find_binder_in_pattern`, resolve
            // Case 6). Reuses the same walker as the tuple case; the head-vs-compound-alias exclusion in
            // `find_binder_in_pattern` keeps a `(list …)`/`(tuple …)` LHS out (those are the tuple case
            // above / a list binding). Zero new IR — the binding position IS a one-arm irrefutable match.
            else if find_binder_in_pattern_is_ctor(db, lhs) {
                let mut path = Vec::new();
                let mut heads = Vec::new();
                if find_binder_in_pattern(db, lhs, name, &mut path, &mut heads) {
                    return Some(Resolved::SumPayload {
                        scrutinee: kv[1],
                        steps: path.into(),
                        heads: heads.into(),
                    });
                }
            }
        }
    }
    None
}

/// Whether `lhs` is a CONSTRUCTOR-headed destructuring pattern `(Ctor arg…)` — a `(. Sum V)` member head
/// or a bare variant name, but NOT a compound-value alias (`list`/`tuple`/`record`/`map`, which have
/// their own binder cases). The gate for routing a `let` binding LHS through `find_binder_in_pattern`
/// (the variant-payload walker) rather than the tuple/name paths. Mirrors `find_binder_in_pattern`'s own
/// head check so the two agree on what is a constructor pattern.
fn find_binder_in_pattern_is_ctor(db: &Db, lhs: StructId) -> bool {
    let Struct::List(app) = db.ast.get(lhs) else {
        return false;
    };
    let Some(&head) = app.first() else {
        return false;
    };
    if app.len() < 2 {
        return false; // a lone head / nullary pattern binds no payload
    }
    let is_compound_alias = db
        .ast
        .as_name(head)
        .is_some_and(|h| matches!(h, "list" | "tuple" | "record" | "map"));
    !is_compound_alias && (db.ast.as_form(head, ".").is_some() || db.ast.as_name(head).is_some())
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

/// Resolve `(do f1 f2 … fn)` in EXPRESSION position — a SEQUENCING block whose value is its LAST form
/// (`core-semantics.md` §A Sequencing Block Yields Its Last Form). This increment realizes the
/// VALUE-ONLY block: every non-last form is a pure value expression, EVALUATED for its (discarded)
/// value and the block's value is `fn`. Since the compiler folds constants and has no effects yet, a
/// pure discarded intermediate contributes nothing — so the resolved form is simply the LAST form's
/// resolved value (a `Ref` to `fn`), and the intermediates are still type-checked / trap-walked via the
/// fault pass reaching them through this node's descent. A block containing an in-block `(def …)`
/// DECLINES for now (the def-in-do binding-scope is a follow-up increment) rather than silently dropping
/// the declaration; an empty `(do)` is malformed.
fn resolve_do(db: &Db, id: StructId) -> Resolved {
    let forms = db.ast.as_form(id, "do").unwrap_or(&[]);
    let Some((&last, _)) = forms.split_last() else {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            "an empty `do` block has no value",
        ));
    };
    // A do-local `(def …)` DECLARATION binds its name for the forms that FOLLOW it (`core-semantics.md`
    // §A Declaration In A Sequencing Block Is Scoped To The Forms That Follow It) — resolved lazily by
    // `binder_in`'s do-case (the same way a top-level/module def is resolved by name), sequentially like
    // a `let` (a form sees only the declarations BEFORE it). But a sequencing block YIELDS its last
    // form's value, so the last form must be a value expression: a TRAILING declaration leaves the block
    // valueless (`(do (def x 5))` has nothing to yield).
    if matches!(
        db.ast.head_name(last),
        Some("def") | Some("type") | Some("effect")
    ) {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            "a `do` block must end in a value form, not a declaration",
        ));
    }
    // The block's value IS the last form's value — a `Ref` to it (the same shape a bare name takes to
    // its binding). The earlier forms are declarations (bound by `binder_in`) or pure statements (their
    // value discarded); either way the fault walk still reaches them through this node's `do`-descent so
    // an ill-typed or provably-trapping form is caught (`infer::collect_node`/`collect_reached_poisons`).
    Resolved::Ref { value: last }
}

/// If `def_form` is a do-local `(def SIG BODY)` declaration binding `name`, the value the name denotes —
/// a `Ref` to the value (a VALUE declaration `(def x V)` or a nullary `(def (x) V)`) or a `Lambda` (a
/// FUNCTION declaration `(def (f p…) BODY)`). Mirrors [`def_as_resolved`] for a declaration reached
/// through a sequencing block rather than the top-level scan; the bare-name value form `(def x V)` is a
/// do/module surface the top-level scan does not carry. `None` if `def_form` is not a `def`, is bodyless
/// (malformed — no binding), or does not bind `name`.
fn do_def_binds(db: &Db, def_form: StructId, name: &str) -> Option<Resolved> {
    let tail = db.ast.as_form(def_form, "def")?;
    let sig = *tail.first()?;
    let body = *tail.get(1)?;
    // Bare-name value declaration `(def x V)` — the name denotes its value.
    if let Some(n) = db.ast.as_name(sig) {
        return (n == name).then_some(Resolved::Ref { value: body });
    }
    // List signature `(NAME param…)`: a nullary `(def (x) V)` denotes its body; a `(def (f p…) BODY)`
    // with parameters denotes the lambda `(p…) BODY` (applied by the ordinary application path). The
    // params are the raw signature occurrences after NAME (bare `a` or annotated `(: a T)`), exactly the
    // shape `def_as_resolved` builds from a scanned def's `params`.
    if let Struct::List(children) = db.ast.get(sig) {
        let def_name = children.first().and_then(|&c| db.ast.as_name(c))?;
        if def_name != name {
            return None;
        }
        let params: Vec<StructId> = children[1..].to_vec();
        if params.is_empty() {
            return Some(Resolved::Ref { value: body });
        }
        return Some(Resolved::Lambda {
            params: params.into(),
            body,
        });
    }
    None
}

/// The value occurrence a do-local VALUE declaration binds — `V` in `(def x V)` or the body of a nullary
/// `(def (x) V)`. `None` for a FUNCTION declaration `(def (f p…) BODY)` (its body is checked on CALL, via
/// β-reduction of a reference — an uncalled lambda body is not eagerly checked, matching a `let`-bound
/// lambda) or a malformed def. The fault walks use this to type-check a value declaration's value eagerly
/// (a value binding's value is a fault whether or not the name is used, like a `let` binding value).
pub(crate) fn do_value_def_value(db: &Db, def_form: StructId) -> Option<StructId> {
    let tail = db.ast.as_form(def_form, "def")?;
    let sig = *tail.first()?;
    let body = *tail.get(1)?;
    if db.ast.as_name(sig).is_some() {
        return Some(body); // `(def x V)`
    }
    if let Struct::List(children) = db.ast.get(sig)
        && children.len() == 1
    {
        return Some(body); // nullary `(def (x) V)`
    }
    None
}

/// Resolve `(if COND THEN ELSE)`.
/// A FIXED-ARITY grammar form's wrong-arity reject, anchored at the form `id`, with a surplus-delete
/// fix when there are TOO MANY operands: delete the FIRST extra (`tail[want]`), the same surplus-arg
/// delete fix an over-applied operator / a too-many-operand quote gets — `cdz fix --all` removes extras
/// until exactly `want` remain. TOO FEW carries NO fix (nothing to delete; supplying an operand is not a
/// mechanical edit). Shared by `if` (want 3), `and`/`or` (want 2), `not` (want 1).
fn fixed_arity_reject(id: StructId, tail: &[StructId], want: usize, message: &str) -> Reject {
    let reject = Reject::coded(Code::Malformed, message.to_string()).at(id);
    match tail.get(want) {
        Some(&surplus) => reject.with_fix(crate::diag::Fix::delete_heuristic(
            surplus,
            "remove the extra operand",
        )),
        None => reject,
    }
}

fn resolve_if(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "if").unwrap_or(&[]);
    if tail.len() != 3 {
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            3,
            "if takes exactly 3 operands",
        ));
    }
    Resolved::If {
        cond: tail[0],
        then_: tail[1],
        else_: tail[2],
    }
}

/// Resolve `(and A B)` / `(or A B)` into a short-circuiting connective (`is_and` picks which). Exactly
/// two operands; a wrong arity is malformed (CDZ0201). The operands stay AST occurrences resolved on
/// demand — the RIGHT one only reached on the non-short-circuit branch at emit, so a trapping `B` is
/// shielded (core-semantics.md §Boolean Connectives Short-Circuit).
///
/// This arm (with `resolve_not` below) is where the language offers the three boolean connectives —
/// conjunction `(and …)`, disjunction `(or …)`, and negation `(not …)` — so a program composes
/// conditions directly rather than nesting one conditional per condition.
//= spec/capabilities/core-semantics.md#boolean-connectives-short-circuit
//# The language MUST offer a logical conjunction, a logical disjunction, and a logical negation over boolean values, so that a program composes conditions without nesting a conditional per condition.
fn resolve_connective(db: &Db, id: StructId, is_and: bool) -> Resolved {
    let head = if is_and { "and" } else { "or" };
    let tail = db.ast.as_form(id, head).unwrap_or(&[]);
    if tail.len() != 2 {
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            2,
            &format!("{head} takes exactly 2 operands"),
        ));
    }
    Resolved::And {
        lhs: tail[0],
        rhs: tail[1],
        is_and,
    }
}

/// Resolve `(not A)` into a logical negation. Exactly one operand; a wrong arity is malformed (CDZ0201).
fn resolve_not(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "not").unwrap_or(&[]);
    if tail.len() != 1 {
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            1,
            "not takes exactly 1 operand",
        ));
    }
    Resolved::Not { operand: tail[0] }
}

/// Resolve `(quote FORM)` — the non-strict quotation form. `quote` requires EXACTLY ONE operand: the form
/// it denotes. This increment handles only the DEGENERATE arity check — a zero-operand `(quote)` has
/// nothing to quote, so it is MALFORMED (CDZ0201, 07-type-system "an empty quote is rejected, not a
/// crash"): the compiler rejects it rather than panicking reaching for the absent quoted node. A
/// well-formed `(quote FORM)` (one operand) DECLINES — building the `Ast` value it denotes is the
/// metaprogramming vertical (12-metaprogramming), not yet realized — so it is a Todo, never a miscompile.
fn resolve_quote(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "quote").unwrap_or(&[]);
    if tail.len() != 1 {
        return Resolved::Poison(arity_one_operand_reject(
            id,
            tail,
            "quote takes exactly one operand — the form it denotes",
        ));
    }
    Resolved::Poison(Reject::decline(
        "quote produces an AST value (not yet built)",
    ))
}

/// The "takes exactly one operand" reject shared by `quote`/`quasiquote`/`unquote`/`unquote-splicing`,
/// anchored at the form `id`. When there are TOO MANY operands (`(quote 1 2)`), the mechanical repair is
/// to DELETE the surplus — remove the SECOND operand (`tail[1]`), the first extra, so `cdz fix --all`
/// converges (each pass removes one until exactly one remains), mirroring the operator-arity surplus-arg
/// delete fix. A ZERO-operand form (`(quote)`) carries NO fix — there is nothing to delete, and inserting
/// a form the author must supply is not a mechanical edit. Heuristic: which surplus to drop is a guess,
/// but removing this one moves toward the one-operand shape.
fn arity_one_operand_reject(id: StructId, tail: &[StructId], message: &'static str) -> Reject {
    let reject = Reject::coded(Code::Malformed, message).at(id);
    match tail.get(1) {
        Some(&surplus) => reject.with_fix(crate::diag::Fix::delete_heuristic(
            surplus,
            "remove the extra operand",
        )),
        None => reject,
    }
}

/// Resolve `(unquote e)` / `(unquote-splicing e)` — the quasiquote escapes (reader-desugared from `,e` /
/// `,@e`). Two well-formedness checks (`metaprogramming.md` §Quasiquote Constructs AST With Selective
/// Evaluation):
///  - ARITY: `unquote`/`unquote-splicing` take EXACTLY ONE operand — the expression to evaluate and
///    embed. Any other count (`(unquote 1 2)`, `(unquote)`) is MALFORMED (CDZ0201), never silently
///    truncated to the first operand. Checked FIRST, so a wrong-arity form inside a quasiquote is the
///    arity error, not the context error.
///  - CONTEXT: an escape is meaningful ONLY inside a `` ` `` template. One OUTSIDE any quasiquote — a bare
///    `,x`, or one nested only under a PLAIN `(quote …)` (a quote body is inert data, NOT a
///    selective-evaluation template) — has no template to insert into → a SYNTAX error (CDZ0003).
///
/// A well-formed escape genuinely INSIDE a quasiquote DECLINES (its meaning is realized by the
/// `Ast`-construction vertical, not yet built) — a Todo, never a miscompile.
fn resolve_unquote(db: &Db, id: StructId, head: &str) -> Resolved {
    let tail = db.ast.as_form(id, head).unwrap_or(&[]);
    if tail.len() != 1 {
        // `head` is `unquote`/`unquote-splicing` (a `&'static str` — the grammar heads), so the message
        // formats to a `'static`-lived string only for the leak-free lifetime the helper needs; build the
        // reject inline here (the message interpolates `head`) but reuse the surplus-delete shape.
        let reject = Reject::coded(
            Code::Malformed,
            format!("{head} takes exactly one operand — the expression to evaluate and embed"),
        )
        .at(id);
        return Resolved::Poison(match tail.get(1) {
            Some(&surplus) => reject.with_fix(crate::diag::Fix::delete_heuristic(
                surplus,
                "remove the extra operand",
            )),
            None => reject,
        });
    }
    if !inside_quasiquote(db, id) {
        return Resolved::Poison(Reject::coded(
            Code::UnquoteOutsideQuasiquote,
            format!(
                "{head} is only valid inside a quasiquote template (a `,`/`,@` outside a `` ` `` has no \
                 template to insert into; a plain `(quote …)` body is inert data, not a template)"
            ),
        ));
    }
    Resolved::Poison(Reject::decline(
        "an unquote inside a quasiquote builds an AST value (not yet built)",
    ))
}

/// Resolve `(quasiquote FORM)` — the selective-evaluation template (reader-desugared from `` `FORM ``).
/// Requires exactly one operand (a zero/multi-operand quasiquote is malformed, like `quote`). A
/// well-formed quasiquote DECLINES — building the `Ast` value (expanding its `unquote`/`unquote-splicing`
/// escapes) is the metaprogramming vertical, not yet realized.
fn resolve_quasiquote(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "quasiquote").unwrap_or(&[]);
    if tail.len() != 1 {
        return Resolved::Poison(arity_one_operand_reject(
            id,
            tail,
            "quasiquote takes exactly one operand — the template it denotes",
        ));
    }
    Resolved::Poison(Reject::decline(
        "quasiquote produces an AST value (not yet built)",
    ))
}

/// Whether the form at `id` is lexically ENCLOSED by a `(quasiquote …)` — the context in which an
/// `unquote`/`unquote-splicing` is meaningful. Walks parents to the root; a `(quasiquote …)` ancestor
/// answers yes. A plain `(quote …)` ancestor does NOT count — quote suppresses evaluation, it does not
/// open a selective-evaluation template (`metaprogramming.md` §Quote Produces An AST Value: a quote body
/// is inert data). This is the whether-inside test, not the nesting-LEVEL count a full quasiquote
/// expander tracks (nested `` ` ``/`,` levels) — sufficient for the syntax rejection this increment makes.
fn inside_quasiquote(db: &Db, id: StructId) -> bool {
    let mut cursor = db.parent_of(id);
    while let Some(form) = cursor {
        if db.ast.head_name(form) == Some("quasiquote") {
            return true;
        }
        cursor = db.parent_of(form);
    }
    false
}

/// Resolve `(match SCRUTINEE (PATTERN BODY)…)` into its resolved form. The scrutinee and each arm's
/// pattern/body stay AST occurrences (resolved on their own demand); this records only the shape. Each
/// arm must be a two-element `(pattern body)` list. A `match` with no arms is malformed.
fn resolve_match(db: &Db, id: StructId) -> Resolved {
    const SHAPE: &str = "a match is `(match <scrutinee> (<pattern> <body>)…)`";
    let tail = db.ast.as_form(id, "match").unwrap_or(&[]);
    let scrutinee = match tail.first() {
        Some(&s) => s,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this match has no scrutinee — {SHAPE}"),
            ));
        }
    };
    let mut arms: Vec<(StructId, StructId)> = Vec::new();
    for &arm in &tail[1..] {
        match db.ast.get(arm) {
            Struct::List(pb) if pb.len() == 2 => arms.push((pb[0], pb[1])),
            _ => {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    format!("a match arm must be `(<pattern> <body>)` — {SHAPE}"),
                ));
            }
        }
    }
    // A ZERO-ARM match is NOT malformed: it is the degenerate exhaustiveness base case, valid when the
    // scrutinee is UNINHABITED (`Never` — a diverging expression; `type-system.md §Never Is The Empty
    // Sum`). Whether THIS scrutinee is actually uninhabited is a downstream verdict (`lower_match` returns
    // the scrutinee's divergence, or `Code::NonExhaustive` if it has values), not a syntactic one — so the
    // shape resolves and the type/lowering stages decide, rather than a blanket "no arms" rejection here.
    Resolved::Match { scrutinee, arms }
}

/// Resolve `(bin <segment>…)` — the dual-direction binary form. Each segment is written like a
/// constructor `(<kind> <slot> <modifier>…)`: an integer `(uNN v)`/`(iNN v)` (big-endian, `le` modifier
/// for little-endian), a bit-field `(bits v k)` (k a compile-time constant), or a byte splice/bind
/// `(bytes b [n])`. The kind/width/sign/endian are decided HERE from the head name (a fixed, closed set —
/// like the grammar heads); the `slot` value stays an AST occurrence (a constant to encode when building,
/// a binder/literal probe when matching — the direction is decided downstream). An unrecognized segment
/// head or a malformed segment is a `Poison`. `(bin)` (no segments) is the empty byte sequence.
fn resolve_bin(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "bin").unwrap_or(&[]);
    let mut segs = Vec::with_capacity(tail.len());
    for &seg in tail {
        // Each segment is a list `(<kind> <slot> [modifier]…)`.
        let Struct::List(parts) = db.ast.get(seg) else {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                "a bin segment must be (<kind> <slot> [modifier])",
            ));
        };
        let Some(kind_name) = parts.first().and_then(|&h| db.ast.as_name(h)) else {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                "a bin segment needs a kind head (uNN/iNN/bits/bytes)",
            ));
        };
        // A fixed-width integer segment: `(uNN v [le])` / `(iNN v [le])`. Width in BYTES from the name.
        let int_kind = match kind_name {
            "u8" => Some((1u8, false)),
            "u16" => Some((2, false)),
            "u32" => Some((4, false)),
            "u64" => Some((8, false)),
            "i8" => Some((1, true)),
            "i16" => Some((2, true)),
            "i32" => Some((4, true)),
            "i64" => Some((8, true)),
            _ => None,
        };
        if let Some((width, signed)) = int_kind {
            if parts.len() < 2 {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "an integer bin segment needs a value: (uNN v [le])",
                ));
            }
            let slot = parts[1];
            // The only modifier on an int segment is `le` (little-endian). Anything else is malformed.
            let mut little_endian = false;
            for &m in &parts[2..] {
                match db.ast.as_name(m) {
                    Some("le") => little_endian = true,
                    _ => {
                        return Resolved::Poison(Reject::coded(
                            Code::Malformed,
                            "the only integer bin-segment modifier is `le`",
                        ));
                    }
                }
            }
            segs.push(crate::resolved::Segment {
                kind: crate::resolved::SegKind::Int { width, signed },
                slot,
                little_endian,
            });
            continue;
        }
        // A bit-field `(bits v k)` — `k` a compile-time constant width (read as a literal here; a
        // non-constant width is CDZ0220, checked at infer where the evaluator is available).
        if kind_name == "bits" {
            if parts.len() != 3 {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "a bit-field bin segment is (bits v k)",
                ));
            }
            let slot = parts[1];
            // `k` must be an integer literal (a constant width). A non-literal is left to infer's CDZ0220.
            let k = match db.ast.get(parts[2]) {
                Struct::Atom(l) => match db.ast.leaf(*l) {
                    Leaf::Int { value, .. } => value.to_i64().filter(|n| *n >= 0).map(|n| n as u32),
                    _ => None,
                },
                _ => None,
            };
            let Some(k) = k else {
                return Resolved::Poison(Reject::coded(
                    Code::IllFormedBinary,
                    "a bit-field width must be a compile-time constant natural",
                ));
            };
            segs.push(crate::resolved::Segment {
                kind: crate::resolved::SegKind::Bits { k },
                slot,
                little_endian: false,
            });
            continue;
        }
        // A byte-sequence segment `(bytes b [n])` — splice all of `b` (build) / bind rest-or-exactly-n
        // (match). `n` (the dependent size) stays an occurrence, resolved downstream.
        if kind_name == "bytes" {
            if parts.len() < 2 || parts.len() > 3 {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "a bytes bin segment is (bytes b) or (bytes b n)",
                ));
            }
            let slot = parts[1];
            let size = parts.get(2).copied();
            segs.push(crate::resolved::Segment {
                kind: crate::resolved::SegKind::Bytes { size },
                slot,
                little_endian: false,
            });
            continue;
        }
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            "an unrecognized bin segment kind (expected uNN/iNN/bits/bytes)",
        ));
    }
    Resolved::Bin { segs }
}

/// Resolve the PIPELINE operator `(|> L R)` into an ordinary application that threads `L` as `R`'s
/// FIRST argument. Exactly two operands; a wrong arity is malformed (CDZ0201).
///
/// The rewrite is purely structural and reuses the EXISTING `L`/`R` occurrences (no AST synthesis, no
/// re-parenting): downstream sees a plain `Resolved::Apply`, so inference, lowering, and every backend
/// need no `|>` awareness. Because scope is derived by walking AST parents (`prelude-and-resolution.md`
/// §Scope Is Found By Walking Parents) and this rewrite moves no node, `L` still resolves in the scope
/// where it is written and `R`'s arguments in theirs.
///
/// Two shapes:
///   - `R` is an APPLICATION form `(f a…)` — a name/expr-headed list, NOT a compound-value literal
///     (`("list" …)`/`("record" …)`/…) or a grammar form (`(. o k)`/`(if …)`/…). Splice: `L |> f(a)`
///     → `(f L a)`.
///   - Anything else (a bare name `f`, a member access `o.m`, a `(f)` — handled by the first arm — or a
///     non-callable value): apply `R` to `L`. `L |> f` → `(f L)`; `L |> o.m` → `(o.m L)`. A `R` that is
///     not applyable (`L |> [1, 2]`) becomes a poison downstream through the ordinary meta-apply channel,
///     the same diagnostic a bare `([1, 2] L)` would give.
fn resolve_pipeline(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "|>").unwrap_or(&[]);
    if tail.len() != 2 {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            "`|>` takes exactly 2 operands: a value and the function to pipe it into",
        ));
    }
    let (lhs, rhs) = (tail[0], tail[1]);
    // Splice into `rhs` when it is an application form: a non-empty list whose head is neither a
    // compound-value constructor (a string head) nor a grammar form — i.e. exactly the shape `compute`
    // classifies as `Resolved::Apply`.
    if let Struct::List(children) = db.ast.get(rhs)
        && !children.is_empty()
        && db.ast.head_ctor(rhs).is_none()
        && db.ast.head_name(rhs).is_none_or(|h| !is_grammar_head(h))
    {
        let head = children[0];
        let mut args = Vec::with_capacity(children.len());
        args.push(lhs);
        args.extend_from_slice(&children[1..]);
        return Resolved::Apply {
            head,
            args: args.into(),
        };
    }
    // Otherwise apply `rhs` itself to `lhs` (a bare function name, a projected method, …).
    Resolved::Apply {
        head: rhs,
        args: std::sync::Arc::from([lhs]),
    }
}

/// The full canonical handler SURFACE shape, shown in every malformed-handle message so a truncated or
/// legacy form teaches the author every part it needs. Vocab is the surface's: the state the handler
/// establishes is the "seed" (`capabilities-and-effects.md` §A Handler Discharges Its Effect / the
/// corpus header), never the internal "init state".
const HANDLE_SHAPE: &str =
    "a handle is `(handle <effect> <seed> ((<op> (params…) <state> <body>)…) <body>)`";

/// Reject a node still headed `handle` at resolve time. `effects::desugar_handles` re-spells every
/// CANONICAL 5-child handle to the internal head, so a leftover `handle` is NOT canonical — it is
/// either the retired effect-name-less shape `(handle <seed> (arm…) <body>)` (the effect must now be
/// named in the head, and each arm's op written bare) or a too-short/malformed handle. One canonical
/// way to write a handler; this is not it.
fn resolve_noncanonical_handle(_db: &Db, _id: StructId) -> Resolved {
    Resolved::Poison(Reject::coded(
        Code::Malformed,
        format!(
            "{} — name the effect in the head and write each arm's operation bare. {HANDLE_SHAPE}",
            crate::diag::HANDLE_NONCANONICAL_PREFIX
        ),
    ))
}

/// Resolve the internal `(handle-internal INIT (ARM…) BODY)` into its resolved form — the node
/// `effects::desugar_handles` produces from a canonical `(handle E seed (bare-op-arm…) body)`. Each arm
/// is `(op-proj (params…) state body)` — the operation projection, a parenthesized parameter list, the
/// state binder, and the arm body. Scope for the params/state binders is handled by the ordinary
/// parent-walk (a reference in the arm body finds its binder), so here we only record the shape. A
/// malformed arm or a missing init/body is a `Poison`.
fn resolve_handle(db: &Db, id: StructId) -> Resolved {
    // A too-short internal tail is reported as incomplete rather than mis-enumerated (we cannot reliably
    // name WHICH part is missing); the shape carries the fix.
    const SHAPE: &str = HANDLE_SHAPE;
    let tail = db
        .ast
        .as_form(id, crate::effects::HANDLE_INTERNAL)
        .unwrap_or(&[]);
    let init = match tail.first() {
        Some(&s) => s,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this handle is empty — {SHAPE}"),
            ));
        }
    };
    let arms_occ = match tail.get(1) {
        Some(&a) => a,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this handle is incomplete — {SHAPE}"),
            ));
        }
    };
    let body = match tail.get(2) {
        Some(&b) => b,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this handle is incomplete — {SHAPE}"),
            ));
        }
    };
    // The arms list `((E.op (p…) s body) …)`. Each arm has FOUR parts: op projection, param list, state
    // binder, arm body.
    let Struct::List(arm_nodes) = db.ast.get(arms_occ) else {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!("this handle's arms must be a list of arms — {SHAPE}"),
        ));
    };
    let mut arms = Vec::new();
    for &arm in arm_nodes {
        let Struct::List(parts) = db.ast.get(arm) else {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                "a handle arm must be `(<op> (params…) <state> <body>)`",
            ));
        };
        if parts.len() != 4 {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                "a handle arm must be `(<op> (params…) <state> <body>)`",
            ));
        }
        let op = parts[0];
        let params = match db.ast.get(parts[1]) {
            Struct::List(ps) => ps.clone(),
            // A single bare param (no parens) — treat as one param. `()` is the empty list (nullary).
            _ => vec![parts[1]],
        };
        arms.push(HandleArm {
            op,
            params,
            state: parts[2],
            body: parts[3],
        });
    }
    if arms.is_empty() {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!(
                "this handle has an empty arm list — a handler must bind every operation its effect declares. {SHAPE}"
            ),
        ));
    }
    Resolved::Handle {
        init,
        arms: arms.into(),
        body,
    }
}

/// Resolve `(resume VALUE NEXT-STATE)` into its resolved form. The two children are AST occurrences
/// resolved on demand. Meaningful only inside a handler arm (the enclosing arm's lowering consumes it);
/// a stray `resume` declines at lowering. A missing value or next-state is a `Poison`.
fn resolve_resume(db: &Db, id: StructId) -> Resolved {
    const SHAPE: &str = "a resume is `(resume <value> <next-state>)`";
    let tail = db.ast.as_form(id, "resume").unwrap_or(&[]);
    let value = match tail.first() {
        Some(&v) => v,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this resume has no value or next-state — {SHAPE}"),
            ));
        }
    };
    let next_state = match tail.get(1) {
        Some(&s) => s,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this resume has no next-state — {SHAPE}"),
            ));
        }
    };
    Resolved::Resume { value, next_state }
}

/// Resolve `(host (EFFECT…) BODY)` into its resolved form — an entrypoint delegation. `effects` are the
/// delegated effects' name occurrences; `body` the delegated computation. A missing effect list or body
/// is a `Poison`.
fn resolve_host(db: &Db, id: StructId) -> Resolved {
    const SHAPE: &str = "a host is `(host (<effect>…) <body>)`";
    let tail = db.ast.as_form(id, "host").unwrap_or(&[]);
    let effects_occ = match tail.first() {
        Some(&e) => e,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this host has no effect list and no body — {SHAPE}"),
            ));
        }
    };
    let body = match tail.get(1) {
        Some(&b) => b,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this host has no body — {SHAPE}"),
            ));
        }
    };
    let effects = match db.ast.get(effects_occ) {
        Struct::List(es) => es.clone(),
        _ => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this host's effects must be a list — {SHAPE}"),
            ));
        }
    };
    Resolved::Host { effects, body }
}

/// Resolve `(let (BINDINGS) BODY)` into its resolved form. The bindings are `(name init)` pairs; scope
/// is handled by the parent-walk (a reference finds its binder), so here we only record the shape.
fn resolve_let(db: &Db, id: StructId) -> Resolved {
    const SHAPE: &str = "a let is `(let ((<name> <init>)…) <body>)`";
    let tail = db.ast.as_form(id, "let").unwrap_or(&[]);
    let bindings_occ = match tail.first() {
        Some(&b) => b,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this let has no bindings and no body — {SHAPE}"),
            ));
        }
    };
    let body = match tail.get(1) {
        Some(&b) => b,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this let has no body — {SHAPE}"),
            ));
        }
    };
    let pairs = binding_pairs(db, bindings_occ);
    if pairs.is_empty() {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!("this let's bindings are malformed — each must be `(<name> <init>)`. {SHAPE}"),
        ));
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
/// Read a record row-operation's LITERAL field-name list operand `(a c)` into its labels — the ONE rule
/// shared by `Record.project`/`without`/… (`type-system.md` §A Record Row Is Reshaped Only Through An
/// Explicit Operation). The operand is a bare list whose elements are field NAMES, read as labels via
/// [`read_key`] exactly as a `record` literal's field names are (`prelude-and-resolution.md` §A Member
/// Key Is A Label, Not A Value) — NOT an evaluated value, so `(a c)` is never resolved as an application
/// of `a` to `c`. `None` if the operand is not a list, or any element is not a bare label (a malformed
/// operand — the op then declines rather than reading a partial label set).
pub(crate) fn record_op_labels(db: &Db, node: StructId) -> Option<Vec<Symbol>> {
    let Struct::List(items) = db.ast.get(node) else {
        return None;
    };
    let mut labels = Vec::with_capacity(items.len());
    for &item in items {
        labels.push(read_key(db, item)?);
    }
    Some(labels)
}

/// A record row operation's BARE field-name operand (`Record.pop r z`) as a label — the `read_key` rule,
/// exposed for the row ops. `None` if `node` is not a bare label (a malformed operand).
pub(crate) fn read_label(db: &Db, node: StructId) -> Option<Symbol> {
    read_key(db, node)
}

/// A record row operation's `(name value)` PAIR operand (`Record.extend`/`Record.with`'s `(z v)`) as
/// `(label, value-occurrence)`. The name is a LABEL (via `read_key`); the value is an ordinary expression
/// (its type/value flow normally — unlike `project`/`without`'s inert label list). `None` if `node` is not
/// a two-element `(name value)` list with a bare-label name (a malformed operand).
pub(crate) fn record_op_pair(db: &Db, node: StructId) -> Option<(Symbol, StructId)> {
    let Struct::List(items) = db.ast.get(node) else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    let label = read_key(db, items[0])?;
    Some((label, items[1]))
}

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
            "Bytes" => Some(Ty::Bytes),
            "String" => Some(Ty::String),
            // `Char`/`Symbol` — the other monomorphic leaf types (like `Bytes`/`String`). Without these
            // arms the bare name decoded to `None`, so a `Char`/`Symbol` NESTED in a compound type-value
            // (a `(Tuple Char …)` element or a variant payload — which boxes as a tuple) round-tripped to
            // `Unit` and mis-typed ("Char and Unit must be the same type"). Paired with `encode_ty`'s
            // `Char`/`Symbol` arms so the round-trip is faithful, exactly as the `Bytes`/`String` arms are.
            "Char" => Some(Ty::Char),
            "Symbol" => Some(Ty::Symbol),
            // `BigInt` — the arbitrary-precision integer leaf, paired with `encode_ty`'s `BigInt` arm so
            // a `BigInt` nested in a compound type-value round-trips faithfully (not collapsing to `Unit`).
            "BigInt" => Some(Ty::BigInt),
            "Float32" => Some(Ty::Float(crate::ty::FloatTy::fixed(32))),
            "Float64" => Some(Ty::Float(crate::ty::FloatTy::fixed(64))),
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
        // A float type-value: `(Float N)` — the dual of `encode_ty`'s `(Float N)` head form. Decodes
        // FAITHFULLY for ANY width, INCLUDING the sentinel 0 a non-admitted `(Float 16)` reduces to, so
        // the width round-trips and the admitted-set check (`infer`'s Annot arm / `reduce_ctor`) is the
        // ONE place a non-admitted float width is rejected — this decoder never drops or remaps a width.
        "Float" => {
            let tail = db.ast.as_form(node, "Float")?;
            let w = tail.first().and_then(|&s| match db.ast.get(s) {
                Struct::Atom(l) => match db.ast.leaf(*l) {
                    Leaf::Int { value, .. } => value.to_i64().and_then(|n| u32::try_from(n).ok()),
                    _ => None,
                },
                _ => None,
            })?;
            Some(Ty::Float(crate::ty::FloatTy::fixed(w)))
        }
        "->" => {
            let tail = db.ast.as_form(node, "->")?;
            // A SINGLE-element arrow `(-> R)` is a nullary type `Unit -> R` — the elided-unit convention
            // (a nullary effect op `(op get (-> R))` performed as `(E.get)`); a two-element `(-> P R)` is
            // the ordinary `P -> R`. (The arrow is strictly binary here — multi-arg curries as nested `->`.)
            match tail.len() {
                1 => {
                    let r = decode_ty(db, tail[0])?;
                    Some(Ty::Fn(Box::new(Ty::Unit), Box::new(r)))
                }
                _ => {
                    let p = decode_ty(db, *tail.first()?)?;
                    let r = decode_ty(db, *tail.get(1)?)?;
                    Some(Ty::Fn(Box::new(p), Box::new(r)))
                }
            }
        }
        "Tuple" => {
            let tail = db.ast.as_form(node, "Tuple")?;
            let mut elems = Vec::with_capacity(tail.len());
            for &e in tail {
                elems.push(decode_ty(db, e)?);
            }
            Some(Ty::Tuple(elems.into()))
        }
        // A list type-value: `(List <elem>)` — the dual of `eval::encode_ty`'s `List` arm.
        "List" => {
            let tail = db.ast.as_form(node, "List")?;
            let elem = decode_ty(db, *tail.first()?)?;
            Some(Ty::List(Box::new(elem)))
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
            let decl = StructId(decl);
            // NEWTYPE NORMALIZATION via the shared `Db::normalize_sum` — the ONE place the `Sum`↔`Nominal`
            // decision + generic template substitution lives, so this wire-decode path agrees with
            // `eval::reduce_sum_ctor` (the generic ctor `(Box Int64)` type-application path). An erasable
            // newtype decl decodes to `Ty::Nominal { inner }` (its stored template with `args` substituted
            // for the param vars); a non-erasable decl stays a boxed `Ty::Sum`.
            Some(db.normalize_sum(decl, name, args))
        }
        // A nominal type-value: `(Nominal <name> <decl> (args…) <inner>)` — the dual of `eval::encode_ty`'s
        // `Nominal` arm. Carries its own `decl + args` (identity) and encoded `inner` (machine-rep hint),
        // so it round-trips independently of `newtype_inner` (an already-built `Ty::Nominal` re-encoded,
        // e.g. through `reduce_ctor`).
        "Nominal" => {
            let tail = db.ast.as_form(node, "Nominal")?;
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
            let args_tail = db.ast.as_form(*tail.get(2)?, "args")?;
            let mut args = Vec::with_capacity(args_tail.len());
            for &a in args_tail {
                args.push(decode_ty(db, a)?);
            }
            let inner = decode_ty(db, *tail.get(3)?)?;
            Some(Ty::Nominal {
                decl: StructId(decl),
                name,
                args,
                inner: Box::new(inner),
            })
        }
        // A map type-value: `(Map K V)` — two type arguments (key first, then value), the dual of
        // `encode_ty`'s `(Map K V)` head and `Ty::render_name`'s `(Map Int64 Int64)` surface. A map's
        // identity is `Map<K,V>` (its key SET is runtime data, not encoded in the type).
        "Map" => {
            let tail = db.ast.as_form(node, "Map")?;
            if tail.len() != 2 {
                return None;
            }
            let k = decode_ty(db, tail[0])?;
            let v = decode_ty(db, tail[1])?;
            Some(Ty::Map(Box::new(k), Box::new(v)))
        }
        // A set type-value: `(Set T)` — one element type (the dual of `eval::encode_ty`'s `(Set T)`).
        "Set" => {
            let tail = db.ast.as_form(node, "Set")?;
            if tail.len() != 1 {
                return None;
            }
            let elem = decode_ty(db, tail[0])?;
            Some(Ty::Set(Box::new(elem)))
        }
        // A quantity type-value: `(Qty <inner-ty> (unit (base NAME EXP)…))` — the dual of
        // `eval::encode_ty`'s `Qty` arm. The inner numeric type then the unit as a `(unit …)` node whose
        // tail is `(base NAME EXP)` triples in canonical (sorted) order (the dimensionless unit is the
        // empty `(unit)`). Rebuilds the canonical `Unit` from the triples via `Unit::mul` of each base
        // raised to its exponent, so the drop-zeros invariant is re-established on decode.
        "Qty" => {
            let tail = db.ast.as_form(node, "Qty")?;
            if tail.len() != 2 {
                return None;
            }
            let inner = decode_ty(db, tail[0])?;
            let unit_items = db.ast.as_form(tail[1], "unit")?;
            let mut unit = crate::ty::Unit::one();
            for &pair in unit_items {
                let items = db.ast.as_form(pair, "base")?;
                if items.len() != 2 {
                    return None;
                }
                let name = db.ast.as_name(items[0])?.to_string();
                let exp = match db.ast.get(items[1]) {
                    Struct::Atom(l) => match db.ast.leaf(*l) {
                        Leaf::Int { value, .. } => value.to_i64()?,
                        _ => return None,
                    },
                    _ => return None,
                };
                unit = unit.mul(&crate::ty::Unit::base(name).pow(exp));
            }
            Some(Ty::Qty {
                inner: Box::new(inner),
                unit,
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
    // A HANDLE-ARM operation parameter: `list` is the arm's `(params…)` list — the 2nd element of a
    // handle arm `(op (params…) state body)` — so `param_node` is one of the operation's formals. Its
    // value is substituted by the perform's argument when the handler resolves; until then it is a formal.
    if is_handle_arm(db, form)
        && let Struct::List(parts) = db.ast.get(form)
        && parts.get(1).copied() == Some(list)
    {
        return true;
    }
    // A HANDLE-ARM STATE binder sits DIRECTLY as the 3rd element of the arm (not inside a list), so
    // `parent` (its immediate parent) is the arm itself. It is a formal like a parameter.
    if is_handle_arm(db, parent)
        && let Struct::List(parts) = db.ast.get(parent)
        && parts.get(2).copied() == Some(id)
    {
        return true;
    }
    false
}

/// Resolve `(fn (param…) body)` into a compile-time lambda. The parameters bind in scope for `body`
/// (the ordinary parameter-scope mechanism, via `binder_in`). A type-lambda like `(fn (a) (-> (Int a)
/// …))` is just this — `a` is an ordinary parameter, not a special "type variable".
fn resolve_lambda(db: &Db, id: StructId) -> Resolved {
    const SHAPE: &str = "a fn is `(fn (<param>…) <body>)`";
    let tail = db.ast.as_form(id, "fn").unwrap_or(&[]);
    let params_occ = match tail.first() {
        Some(&p) => p,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this fn has no parameter list and no body — {SHAPE}"),
            ));
        }
    };
    let body = match tail.get(1) {
        Some(&b) => b,
        None => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this fn has no body — {SHAPE}"),
            ));
        }
    };
    // The parameter occurrences (each a bare name). Collected into the `Arc<[StructId]>` the variant
    // holds (a refcounted slice — cloning the lambda is then O(1)).
    let params: std::sync::Arc<[StructId]> = match db.ast.get(params_occ) {
        Struct::List(ps) => ps.clone().into(),
        _ => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this fn's parameters must be a list — {SHAPE}"),
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
        // A duplicate field name — anywhere in the list — is ill-formed. `field` is the SECOND
        // `(key value)` entry (the first `insert` of this label already succeeded), so the mechanical
        // repair is to DELETE this redundant entry: a record has a fixed field SET, and the earlier entry
        // already binds the name. Anchor at the offending entry and carry a `delete` fix (heuristic — the
        // author might instead have meant to RENAME one field, but removing the duplicate is the direct
        // resolution of "named more than once"; `--verify-fixes` confirms it recompiles).
        if fields.insert(label.clone(), kv[1]).is_some() {
            return Err(Reject::coded(
                Code::Malformed,
                format!("record names field `{}` more than once", label.name),
            )
            .at(field)
            .with_fix(crate::diag::Fix::delete_heuristic(
                field,
                format!("remove the duplicate `{}` field", label.name),
            )));
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

/// Resolve `(list e0 e1 …)` — a homogeneous sequence literal. Every element is an AST occurrence in
/// order (resolved on demand); unlike a tuple the elements are NOT per-position (they all unify to one
/// element type — `infer`/`type_errors` enforce homogeneity). An empty `(list)` has no elements — a
/// list of a deferred element type.
fn resolve_list(db: &Db, id: StructId) -> Resolved {
    let elems: std::sync::Arc<[StructId]> = db.ast.as_ctor_form(id, "list").unwrap_or(&[]).into();
    Resolved::List { elems }
}

/// Resolve `(map (k v) …)` — a persistent key→value association literal. Each entry is a two-element
/// `(key value)` list; UNLIKE a record, BOTH positions are ORDINARY VALUE occurrences (resolved on
/// demand by the normal scope lookup, NOT read as a label via `read_key`) — that is what makes a map
/// key a VALUE (`(let ((a 5)) (map (a 1)))` keys by 5, `(+ 2 3)` is a runtime key, an unbound key is
/// the ordinary CDZ0101). A malformed entry (not a 2-element list — e.g. `(map ("a"))`, a key with no
/// value) is a `Poison` (CDZ0201), never a panic reaching for the absent value. An empty `(map)` is a
/// map with no entries. `infer`/`type_errors` enforce key/value homogeneity + duplicate-const-key.
fn resolve_map(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_ctor_form(id, "map").unwrap_or(&[]);
    let mut entries: Vec<(StructId, StructId)> = Vec::with_capacity(tail.len());
    for &entry in tail {
        match db.ast.get(entry) {
            Struct::List(items) if items.len() == 2 => entries.push((items[0], items[1])),
            _ => {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "a map entry is a (key value) pair",
                ));
            }
        }
    }
    Resolved::Map {
        entries: entries.into(),
    }
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
    use crate::testkit::{if_program, parse, scalar_program};

    /// The `StructId` of the first `(|> …)` occurrence in `db`, for driving `resolve_pipeline`.
    fn pipe_node(db: &Db) -> StructId {
        (0..db.ast.structure.len() as u32)
            .map(StructId)
            .find(|&id| db.ast.head_name(id) == Some("|>"))
            .expect("a |> node")
    }

    #[test]
    fn pipeline_into_a_bare_name_wraps_it_in_a_call() {
        // `(|> x f)` resolves to an application `(f x)` — the piped value becomes the sole argument.
        let ast = parse("(module m (def (main) (|> x f)) (def (f n) n) (export main))");
        let mut db = Db::load(ast);
        let pipe = pipe_node(&db);
        match resolved_of(&mut db, pipe) {
            Resolved::Apply { head, args } => {
                assert_eq!(db.ast.as_name(head), Some("f"));
                assert_eq!(args.len(), 1);
                assert_eq!(db.ast.as_name(args[0]), Some("x"));
            }
            other => panic!("expected Apply, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_into_a_call_splices_the_value_as_the_first_argument() {
        // `(|> x (f a))` resolves to `(f x a)` — the value is threaded as `f`'s FIRST argument, the
        // existing arguments follow. Reuses the source occurrences (no synthesis).
        let ast = parse("(module m (def (main) (|> x (f a))) (def (f p q) p) (export main))");
        let mut db = Db::load(ast);
        let pipe = pipe_node(&db);
        match resolved_of(&mut db, pipe) {
            Resolved::Apply { head, args } => {
                assert_eq!(db.ast.as_name(head), Some("f"));
                assert_eq!(args.len(), 2);
                assert_eq!(db.ast.as_name(args[0]), Some("x")); // spliced first
                assert_eq!(db.ast.as_name(args[1]), Some("a")); // original arg
            }
            other => panic!("expected Apply, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_with_wrong_arity_is_malformed() {
        // A `|>` with one operand cannot thread anything — a coded rejection (CDZ0201), not a silent
        // decline, is what gives the operator its own diagnostic.
        let ast = parse("(module m (def (main) (|> 5)) (export main))");
        let mut db = Db::load(ast);
        let pipe = pipe_node(&db);
        match resolved_of(&mut db, pipe) {
            Resolved::Poison(reject) => assert_eq!(reject.code, Some(Code::Malformed)),
            other => panic!("expected Poison, got {other:?}"),
        }
    }

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
