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
use crate::ast::{CompoundCtor, Leaf, Struct, StructId};
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
//= spec/capabilities/core-semantics.md#a-compound-value-has-a-symbol-constructor-and-a-shadowable-alias
//# A compound value — a tuple, a record — MUST have a **primitive constructor named by a string literal** in head position: a tuple is constructed by `("tuple" …)` and a record by `("record" …)`.
//= spec/capabilities/core-semantics.md#a-compound-value-has-a-symbol-constructor-and-a-shadowable-alias
//# A string literal is not something a name binding can introduce (a binding introduces an identifier, never a string), so the primitive constructor MUST NOT be shadowable, and the language recognizes the string-headed form structurally.
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
    // `(try e)` — the fallible short-circuit operator (`DESIGN-try-operator-rcdzc.md`). CONTROL FLOW,
    // not a strict prelude record: it may abort the enclosing fallible boundary rather than always
    // yielding a value, so it is grammar (like `if`/`match`), dispatched structurally. Kept here so a
    // top-level / leftover `(try …)` is an expression the resolver routes to `resolve_try`, not an
    // unbound name.
    "try",
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
    // Test-only: count every clone-returning call — the noise-free metric for borrow-family cleanups
    // (`resolved_of`→`resolved_ref`), which a fleet-load-swamped wall-clock A/B cannot measure. A
    // PER-`Db` counter (not a process-global atomic) so the parallel test harness's OTHER concurrent
    // compiles can't pollute the reading (`db.rs::Db::resolved_of_calls` doc has the full rationale).
    #[cfg(test)]
    {
        db.resolved_of_calls += 1;
    }
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

/// The resolved form of `id` as a BORROW into the memo column — the zero-clone companion of
/// [`resolved_of`]. `resolved_of` returns by VALUE, so a caller that only needs to READ the resolved
/// form (dispatch on its variant, read a Copy field) pays a full `Resolved` clone per call — and on the
/// hot reducer path (`reduce_to_record_id`/`project_meta`, the `resolved_of` memo-hit clone is ~5% of a
/// realistic compile). This fills the memo on a miss (via `compute`, exactly as `resolved_of` does — same
/// origin-anchoring), then hands back a borrow of the filled slot, so a read-only caller clones nothing.
/// Only for callers that do NOT need `&mut db` while holding the borrow (the borrow ties up `db`).
pub fn resolved_ref(db: &mut Db, id: StructId) -> &Resolved {
    if matches!(db.resolved.get(id), Slot::Filled(_)) {
        // Already memoized — borrow it directly (the common case; no clone, unlike `resolved_of`).
        let Slot::Filled(r) = db.resolved.get(id) else {
            unreachable!("just checked Filled")
        };
        return r;
    }
    // Miss: compute + fill (identical to `resolved_of`'s miss path), then borrow the freshly-filled slot.
    let mut r = compute(db, id);
    if let Resolved::Poison(reject) = &mut r {
        reject.set_origin_if_absent(id);
    }
    db.resolved.fill(id, r);
    let Slot::Filled(r) = db.resolved.get(id) else {
        unreachable!("just filled")
    };
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

/// Drop the memoized RESOLVED form of every node in the subtree rooted at `id` so a following
/// [`resolve_subtree`] RECOMPUTES it against a NEW context. A lowering-time desugar that RE-PARENTS an
/// existing subtree — a map arm BODY moved under a synthesized destructuring `(match __mv (<value-pat>
/// body) …)`, where a value binder resolved to a `MapField` and must now re-resolve against the
/// destructuring pattern — calls this first: `resolved_of` is memoized, so a re-parented node would else
/// keep its stale resolution + the `resolved_subtrees` walk-guard would skip it. Clears both the `resolved`
/// column slot AND the walk-guard for the subtree; the type memo is left (a stale ground type recomputes
/// lazily and re-resolution alone does not invalidate it). Bounded by the subtree size.
pub fn forget_subtree(db: &mut Db, id: StructId) {
    db.resolved.forget(id);
    // The ground type-value memo derives from the resolved form; re-parenting invalidates it too.
    db.typeval.forget(id);
    db.resolved_subtrees.remove(&id);
    if let Struct::List(children) = db.ast.get(id) {
        for c in children.clone() {
            forget_subtree(db, c);
        }
    }
}

/// [`forget_subtree`] that PRESERVES pinned subtrees — a node in [`Db::resolved_subtrees`] (and everything
/// under it) keeps its memoized resolution and walk-guard untouched. Used by the escaped-closure recovery,
/// whose rewritten body mixes two kinds of node. A freshly re-parented ref (`push_list` rebuilt it, or its
/// pre-copy scope memo is now stale) MUST recompute against the rebuilt structure — it is NOT pinned, so
/// this forgets it exactly as [`forget_subtree`] would. A PINNED capture occurrence — a free variable the
/// escaping closure captured, which `pin_free_vars` resolved (via [`resolve_subtree`]) to its real binder
/// and `apply_lambda` then SHARED (not copied) into the reduced body — carries a STALE parent pointer (still
/// into the dead ORIGINAL closure subtree), so forgetting its memo and re-resolving it structurally would
/// scope-walk that dead chain into an orphan → a spurious CDZ0101 (breaker sk4c). Its memoized resolution is
/// already CORRECT (pinned before the re-parent), so preserving it is exactly right — the same "arguments
/// resolve in the caller's scope" invariant `resolve_subtree` exists to protect. A pinned node's
/// `resolved_of` returns its memo before any scope walk, so a later `force_structural_resolution_subtree`
/// over the same tree cannot override it.
pub fn forget_subtree_keep_pinned(db: &mut Db, id: StructId) {
    if db.resolved_subtrees.contains(&id) {
        return;
    }
    db.resolved.forget(id);
    db.typeval.forget(id);
    if let Struct::List(children) = db.ast.get(id) {
        for c in children.clone() {
            forget_subtree_keep_pinned(db, c);
        }
    }
}

/// RE-ANCHOR every `let`-body reference in the subtree at `id` that resolves to the binding initializer
/// `old_init` so it instead resolves to `new_init` — an alpha-consistency repair for a COPIED `let`.
///
/// A `let` reference `x` resolves (via `last_binder_named`) to `Resolved::Ref { value: <init-occ> }` — the
/// INITIALIZER occurrence of its binding, keyed by node identity. When a caller COPIES a `let` (the effect
/// multi-resume peel building a fresh header, or a `deep_fresh_copy` of a `let`-bearing arm body) it mints a
/// FRESH init occurrence `new_init` for the header, but body references it did not itself rebuild — reused
/// branch nodes, or refs whose resolution was memoized against the pre-copy scope — still resolve to the
/// ORIGINAL `old_init`. The kept multi-use `Core::Let` slot is then keyed by `new_init` (the header) while the
/// `Core::LocalRef`s carry `old_init` — which has no slot in the copied scope: "let-binding reference has no
/// local slot" at emit (breaker tpwJ). `copy_structural` avoids this for the `let`s IT builds (its copied refs
/// re-resolve against the copied scope), but a reused foreign ref, or a copy whose refs were resolved before
/// re-parenting, does not — so the caller repairs it here.
///
/// TIMING: call this AFTER the copied tree is re-parented into its lexical site, and SCOPED to the copied
/// `let` body (not a whole wrapped tree), so `resolved_of` on any unrelated node — an enclosing param `n`, a
/// match-pattern binder — resolves against a valid parent chain rather than memoizing a spurious `Unbound`
/// (a pre-reparent or too-wide call poisons those memos — the wiring lesson from the tpwJ prototype).
/// Overwrites the `db.resolved` memo of every node whose resolved form is `Ref { value: old_init }` to
/// `Ref { value: new_init }`. Bounded by the subtree size; a node resolving to any OTHER init (a sibling
/// binding, an outer capture) is untouched, so a well-formed nested/sibling `let` is unaffected.
pub fn reanchor_let_refs(db: &mut Db, id: StructId, old_init: StructId, new_init: StructId) {
    if let Resolved::Ref { value } = resolved_of(db, id)
        && value == old_init
    {
        db.resolved.fill(id, Resolved::Ref { value: new_init });
    }
    if let Struct::List(children) = db.ast.get(id) {
        for c in children.clone() {
            reanchor_let_refs(db, c, old_init, new_init);
        }
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
                } else if is_variant_pattern_binder_occurrence(db, id) {
                    // A VARIANT-PAYLOAD binder occurrence IN THE PATTERN position — the `n` in `((W.V n) …)`
                    // or a nested `((W.V (Option.Some n)) …)`. It NAMES A BINDING, not a value (a body
                    // reference resolves to a `SumPayload` reading the scrutinee's payload, `binder_in` Case
                    // 6). Inert here — the SAME treatment list/map pattern binders get — so an EAGER subtree
                    // walk (`resolve_subtree`, run to pin call-site arguments before β-reduction) does NOT
                    // resolve it via `resolve_name`. Without this, a payload binder that SHADOWS an enclosing
                    // param (`(def (f (: n Int64)) (match … ((W.V n) n) …))`, `f` reached via a CALL) had its
                    // pattern occurrence `n` bound to the PARAM by the eager walk, corrupting the pattern head
                    // detection (`lower` then saw a bound value, not a fresh binder → "not a variant
                    // constructor" / a spurious CDZ0101). A body reference is unaffected (Case 6 still wins,
                    // nearest-scope, giving the payload binder over the param).
                    trace!(target: "rcdzc::resolve", node = id.0, %n, "name → variant-payload binder (inert)");
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
            Leaf::Str(s) => Resolved::Str(s.to_string()),
            // A SYMBOL literal (`#"meter"`) — the reader-sugar equivalent of `(Symbol.of "meter")`
            // (17-symbols). Resolves to a `SymbolConst` typed `Ty::Symbol` (DISTINCT from `Ty::String`, so
            // `(= #"x" "x")` is the nominal-boundary type error CDZ0202 and `(= #"x" (Symbol.of "x"))` is
            // true). Its identity is its text, so it shares the `Core::ConstStr` rep + constant-string
            // equality. `Unit.base #"meter"` still reads its text (a base-dimension name — `unit_of`
            // accepts this form). (Was a `Str` in the Layer-1 units simplification, before `Ty::Symbol`.)
            Leaf::Sym(s) => Resolved::SymbolConst(s.to_string()),
            // A byte-string literal `b"…"` — the reader unescaped it to raw bytes. A `Ty::Bytes`
            // constant (lowers to a `Core::BytesOf` of its bytes, so it bakes/compares/slices exactly
            // like `(Bytes.of (list …))`, and renders back `b"…"`). The companion of the `Str` literal.
            Leaf::Bytes(bs) => Resolved::Bytes(bs.to_vec()),
            // A FLOAT literal — types as `Ty::Float`, distinct from `Ty::Int` (so an int↔float mix
            // rejects, no silent promotion). A literal whose magnitude exceeds the finite Float64 range
            // rounds to `±inf`, which has no written form the reader accepts — so it is a MALFORMED
            // literal (CDZ0201), the float analogue of the out-of-range integer literal
            // `9223372036854775808`. A finite literal resolves to its exact `Decimal`.
            //= spec/capabilities/numeric-model.md#a-floating-point-literal-that-denotes-no-representable-value-is-malformed
            //# A floating-point literal whose magnitude exceeds the largest finite value its type can represent MUST be rejected as a malformed literal at the reader boundary, rather than silently producing a non-finite value that has no written form, exactly as an integer literal outside its type's range is rejected.
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
            // A non-finite float VALUE leaf (NaN, ±∞) — produced by `Ast.encode` of a computed float,
            // never by the reader. In a SOURCE atom position it has no written form the reader accepts,
            // so — exactly like the out-of-Float64-range literal above — it is a malformed literal.
            Leaf::FloatNan | Leaf::FloatInf { .. } => Resolved::Poison(Reject::coded(
                Code::Malformed,
                "non-finite float value has no source literal form".to_string(),
            )),
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
            // A native-compound-data CTOR-HEAD leaf (`Leaf::Ctor`/`FieldPair`/`Member`) is STRUCTURAL — it
            // only ever occupies the HEAD position of a compound `List` node, where the List-level dispatch
            // (`compound_ctor` etc.) reads it. In a bare ATOM (non-head) position it is not a value, so —
            // like a non-finite float leaf above — a stray occurrence is a malformed node.
            Leaf::Ctor(_) | Leaf::FieldPair | Leaf::Member | Leaf::Rational => {
                Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "a compound-constructor head leaf is not a value on its own".to_string(),
                ))
            }
            // A type-suffixed numeric literal (`100N`/`0.5R`) is a SYNTAX-side leaf: the reader desugars a
            // suffixed atom to a `(: <leaf> BigInt|Rational)` annotation and the codec decodes the leaf kind
            // straight to `Int`/`Float`, so a bare `Suffixed` leaf never reaches the decoded compiler AST. A
            // stray occurrence is therefore malformed (like the structural head leaves above).
            Leaf::Suffixed { .. } => Resolved::Poison(Reject::coded(
                Code::Malformed,
                "a type-suffixed literal must be decoded to Int/Float before resolution"
                    .to_string(),
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
            // Recognize the native ctor-LEAF-KIND head (what the reader emits) — unshadowable. Post the M3
            // reader-flip (2026-08-31) the legacy string-primitive head is gone; leaf-only dispatch. (The
            // shadowable NAME alias `(tuple …)`/`(set …)` is NOT recognized here — it resolves lexically.)
            match db.ast.compound_ctor_leaf(id) {
                Some(CompoundCtor::Record) => return resolve_record(db, id),
                Some(CompoundCtor::Tuple) => return resolve_tuple(db, id),
                Some(CompoundCtor::List) => return resolve_list(db, id),
                Some(CompoundCtor::Map) => return resolve_map(db, id),
                Some(CompoundCtor::Set) => return resolve_set(db, id),
                None => {}
            }
            // A native RATIONAL-LITERAL node `(RationalTag n d)` — a `Leaf::Rational` HEAD leaf with two
            // integer children — is what the reader emits for BOTH the bare `3/2` literal and the
            // `#rational(3 2)` alias (v-syntax #6627). Recognize it HERE (list-level, consuming the head
            // leaf like the compound ctors above) so it resolves to a rational CONSTANT and folds to the
            // same `Core::ConstRational` as `(Rational.of n d)`. WITHOUT this it fell to the `Resolved::Apply`
            // path below, whose head is the `Leaf::Rational` atom → the bare-head CDZ0201 at resolve.rs:~435
            // → a bare rational literal was uncompilable on every surface incl. the binary-AST path.
            if children.len() == 3
                && matches!(db.ast.get(children[0]), Struct::Atom(l) if matches!(db.ast.leaf(*l), Leaf::Rational))
            {
                return Resolved::Rational {
                    num: children[1],
                    den: children[2],
                };
            }
            // A native MEMBER-leaf head (`(. obj key)`, what the reader now emits for `.`/`obj.key`) —
            // recognized by leaf kind, not the `.` head text. Dispatch to member access before the
            // name-head match (the legacy `Name(".")` head is still handled by the `Some(".")` arm below).
            if db.ast.member_parts(id).is_some() {
                return resolve_member(db, id);
            }
            match db.ast.head_name(id) {
                Some("if") => resolve_if(db, id),
                Some(h @ ("and" | "or")) => resolve_connective(db, id, h == "and"),
                Some("not") => resolve_not(db, id),
                Some("try") => resolve_try(db, id),
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
                Some("const") => resolve_const_block(db, id),
                // A `(@ …)` annotation node that SURVIVED `strip_annotations` and reached resolve. A
                // well-formed def-wrapping annotation `(@ NAME (def …))` is unwrapped IN PLACE by
                // `strip_annotations` wherever it appears; a TOP-LEVEL survivor (wraps no def) is already
                // rejected by `collect_faults`' `malformed_annotation_forms` scan. What reaches HERE is a
                // NESTED `(@ …)` in expression position — e.g. `(do (: (@ (param …) width) Int64) …)` — which
                // that top-level-only scan misses. Left to fall through, `@` resolves as an "unbound name `@`"
                // and EACH internal token (`param`/`tag`, `widget`/`x`, …) cascades its OWN spurious
                // unbound-name error — a baffling pile for a user who wrote an annotation, not name references
                // (v-metaprogramming diagnostic-quality report, 2026-07-17). Report ONE clean CDZ0201 naming
                // the misplacement, and do NOT resolve the internals (return a Poison so the node is inert).
                Some("@") => Resolved::Poison(
                    Reject::coded(
                        Code::Malformed,
                        "an `(@ …)` annotation cannot appear here — an annotation attaches to a top-level \
                         definition, as `(@ <name> (def …))` (e.g. `(@ test (def (t) 1))`); it is not an \
                         expression and cannot be nested in a `do`-block or other value position",
                    )
                    .at(id),
                ),
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
                    Resolved::Poison(Reject::unsupported(format!(
                        "built-in `{op}` is not supported"
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
                            Resolved::Poison(Reject::decline(
                                crate::diag::UNKNOWN_INTRINSIC_DECLINE,
                            ))
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
        // An INTERNAL effect-specialization (`base#eff<n>`) with no body is a DANGLING reservation: the
        // effect specializer reserves the spec def (body `None`) + memoizes its name BEFORE threading the
        // recursive body, so a self-call can resolve its own name; when the body is UNTHREADABLE (e.g. a
        // do-def-bound perform in the recursive fn — not yet specializable), `thread` returns `None` and the
        // reserved def is left bodyless. A reference to it must NOT surface the mangled internal name via a
        // coded CDZ0201 "`base#eff2` has no body" (a compiler-internal name in a user-facing message,
        // corpus-bugfix/breaker 2026-07-28). Report a CDZ0900 unsupported-construct decline naming the BASE
        // fn instead (seq-286: every decline carries a code; seq-280: the user-facing text is a clean
        // capability statement — the "later increment" framing stays here in-comment, NOT in the message):
        // the cross-function / non-tail resume shape the specializer's body-clone increment will later fold.
        // A genuine user-authored bodyless def keeps its coded CDZ0201 (its name has no `#eff` marker).
        None => {
            let poison = if let Some(base) =
                name.split("#eff").next().filter(|_| name.contains("#eff"))
            {
                Reject::unsupported(format!(
                    "the tail-resumptive fold cannot specialize `{base}`: it performs a discharged \
                     operation via a cross-function or non-tail resume, which the effect specializer \
                     does not lower"
                ))
            } else {
                Reject::coded(Code::Malformed, format!("`{name}` has no body"))
            };
            Resolved::Poison(poison)
        }
    }
}

/// Whether the nullary-def body `body` is a SELF-REFERENTIAL VALUE with no base case — a value defined
/// directly in terms of itself (`(def (g) g)`) or a mutual cycle (`(def (a) b) (def (b) a)`), where each
/// body resolves to a bare `Resolved::Ref` to another (or the same) body, forming a `Ref` cycle that
/// bottoms out in no value. Such a def has no meaning (`g = g` names nothing), and the reduction would
/// spin until the depth guard fires, mislabeling it "expression nests too deeply (a resource limit)".
/// This detects the cycle STRUCTURALLY — follow the `Ref` chain, tracking visited body nodes — WITHOUT
/// reducing, so it names the real fault before the limit is hit.
///
/// Reports `body` only when the chain returns to `body` ITSELF — i.e. `body` is a member of the cycle.
/// A def that merely POINTS INTO a cycle without being part of it — `(def (a) b) (def (b) b)`, where `a`
/// references the self-cyclic `b` — is NOT "defined in terms of itself": `a` is not in its own cycle, so
/// the "`a` is defined in terms of itself" message would be a false positive (the same misfire that flagged
/// `main` in `(def x x) (def (main) x)` — `main` names the broken `x`, but is not itself cyclic). `b`'s own
/// check reports the real cycle. Tracking a `seen` set of ALL visited nodes and reporting on ANY revisit
/// mis-attributed a downstream cycle to the upstream referrer; keying the closure on the START node fixes
/// that. (`a`'s non-termination is a consequence of `b`'s cycle, reported once at `b`, not doubled at `a`.)
///
/// Only a BARE-`Ref` chain is a cycle here: a body that does COMPUTATION (`(def (g) (+ g 1))` — an
/// `Apply`, not a `Ref`) is NOT reported (its non-termination needs reduction analysis to distinguish
/// from a legitimately-deep program), and a recursive FUNCTION (`(def (f n) …)` — params, so `def_as_
/// resolved` yields a `Lambda`, never a bare `Ref`) is untouched. Conservative: reports only an
/// unambiguous value cycle THROUGH `body`, never a false alarm on a well-formed program.
pub(crate) fn value_ref_cycle(db: &mut Db, body: StructId) -> bool {
    let mut cur = body;
    // Bound the walk by the visited-node count so a chain that enters a cycle NOT containing `body`
    // (`a → b → b`) terminates instead of spinning: once we have followed more distinct `Ref` steps than
    // there are nodes, a cycle exists somewhere downstream — but it did not return to `body`, so `body`
    // is not self-referential. `seen` provides that bound and the not-through-`body` exit.
    let mut seen: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    loop {
        match resolved_of(db, cur) {
            Resolved::Ref { value } => {
                if value == body {
                    return true; // the chain returned to the START — `body` is in its own cycle
                }
                // A revisit of a NON-start node means we entered a cycle downstream of `body` that does
                // not include `body` — `body` reaches a non-terminating value but is not itself cyclic.
                if !seen.insert(value) {
                    return false;
                }
                cur = value;
            }
            _ => return false, // the chain reaches a real value (or computation) — not a bare cycle
        }
    }
}

fn resolve_name(db: &Db, id: StructId, name: &str) -> Resolved {
    // KNOWN-LOW-EDGE — the `__`-synth namespace is DELIBERATELY NOT reserved here. A `__`-prefixed name
    // (`__invariant_construct_<T>`, `__invariant_check_<T>`, `__inv_p`, …) denotes a compiler-SYNTHESIZED
    // def; a user CAN hand-write `(__invariant_construct_Percent x)` and it resolves + lowers, and a direct
    // such call with a PERFORM-derived arg UNDER A HANDLE hits an internal "parameter reference has no local
    // slot" (the effects thread-rebuild orphans the synth def's inlined `__inv_p`). It is TEMPTING to reserve
    // the `__` namespace (reject a user ref → clean CDZ0101), but that BREAKS a DELIBERATELY-PINNED corpus
    // contract: `spec/semantics/26-program-conditions.sexp` case "@invariant ESTABLISH Part 2" calls
    // `__invariant_construct_Percent` BY NAME on purpose (its doc: "synthesized UNWIRED here (called by
    // name); wiring `lower_sum_new` to route every `(Percent.Pct x)` through it is the follow-up sub-slice")
    // to pin run-time establish enforcement (mk 50→50, mk 150→trap). A blanket reservation flips that case
    // pass→todo. No NATURAL program reaches the leaked-internal (only a hand-spelled `__`-call does), so per
    // the concierge ruling (2026-08-06) this stays a documented low-edge, NOT a reservation. The REAL fix is
    // to WIRE the `lower_sum_new` establish-divert so `(Percent.Pct x)` always routes through the checked
    // ctor — then the `__`-name call is unnecessary AND reservable, and case 26 updates to the natural path.
    // That is a future establish-wiring slice (shared lower); until then the `__`-perform-under-handle edge
    // is WONTFIX-low. Do NOT re-add a `__`-namespace reject without doing the divert-wiring + corpus update.
    //
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
    // FILE-SCOPED for a linked package: a type name resolves against the reference's OWN file (its own
    // `(type …)` declarations + its imported types), never a sibling file's type. Without this the flat
    // `type_decl_by_name` below let ANY file name ANY sibling's type and construct its variants with no
    // import — no cross-file type privacy at all (`DESIGN-package-linking.md` §8.3 residual (a)). A
    // single-file compile (`file_scoped_type` → `None`) falls straight through to the flat path,
    // byte-identical to before. `Some(Err(()))` — file known, type not visible there — falls through to
    // the effect/variant/prelude steps and then unbound, so a private sibling type is genuinely invisible.
    let scoped_type = if db.is_linked_package() {
        db.file_scoped_type(id, name)
    } else {
        None
    };
    if let Some(Ok(value)) = scoped_type {
        if db.child_ix_of(id) == 0
            && db.is_user_node(id)
            && let Some(ctor) = db.same_name_newtype_ctor(name)
        {
            return Resolved::Ref { value: ctor };
        }
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → file-scoped sum type decl");
        return Resolved::Ref { value };
    } else if scoped_type.is_none()
        && let Some(value) = db.type_decl_by_name(name)
    {
        // Reached only when NOT file-scoped (single-file, or an indeterminate β-copied node); a linked
        // `Some(Err(()))` (file known, type not visible) skips this so a sibling's private type does not
        // leak — it falls through to the effect/variant/prelude steps and then unbound.
        // SAME-NAME NEWTYPE, in application/pattern-HEAD position: `(type UserId (UserId Int64))` binds
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
        // SAME-NAME CTOR in head position → the CONSTRUCTOR (see the block above for the position rule).
        // The `is_user_node` gate distinguishes a real program construct `(Meters a)` from a SYNTHESIZED
        // `sum_applied` type-expr `(Box a)` (a GENERIC same-name sum's ctor-result type, which MUST stay
        // the type record). But a β-COPY of a user construct — `mk`'s body `(Meters a)` inlined at a call
        // site during specialization — is ALSO a synth node (id ≥ user_node_count) in VALUE head position,
        // and the plain `is_user_node` gate wrongly left it the TYPE → spurious CDZ0203 (the breaker's
        // `adv-same-name-ctor-hijacked-by-type` FACE B, a smart-constructor helper). Discriminator: the
        // confusable `sum_applied` synth only exists for a GENERIC (parameterized) same-name sum — a
        // MONOMORPHIC one (`Meters`, `N` with concrete payloads; no type params) has NO such synth, so a
        // synth head-position occurrence of a monomorphic same-name ctor is ALWAYS a value-construct copy.
        // So ALSO fire on a synth node when the same-name sum is monomorphic AND the node is not inside a
        // type-expression subtree (a monomorphic type takes no args, so `(Meters T)` in type position is
        // itself the CDZ0203 error — never a legitimate type-expr — but the `!is_type_expr_node` guard keeps
        // the value-vs-type split honest for a load-time type-expr occurrence).
        //
        // The monomorphic arm cannot cover a GENERIC same-name sum: a generic sum DOES have a confusable
        // `sum_applied` synth `(Box a)` (the ctor-result type re-applied in type position), and firing the
        // ctor on any synth head would corrupt it (the guard test). But a β-COPY of a VALUE construct
        // `(Box k)` — `inner`'s body inlined into a caller (adv-63), or a param'd def whose returned `(Box
        // k)` reaches emit — is ALSO a synth head, and left as the TYPE it rejects CDZ0203 (or emits an
        // invalid module one stage later). Distinguish the two by β-copy PROVENANCE: `copy_structural`
        // records each copied name atom's SOURCE occurrence (`synth_name_origin`), so `source_of_synth`
        // traces an inlined `(Box k)` head back to the author's VALUE-position `Box` (not a type-expr node),
        // whereas the freshly-pushed `sum_applied` `(Box a)` head has NO provenance (`None`) and an inlined
        // type annotation `(: x (Box a))` traces to a source that IS a type-expr node. So fire the ctor when
        // the synth head traces to a user occurrence OUTSIDE a type-expression subtree — the exact "an
        // inlined value construct" set, leaving both the `sum_applied` synth and a copied annotation as the
        // type.
        let inlined_value_construct = db
            .source_of_synth(id)
            .is_some_and(|src| !db.is_type_expr_node(src));
        if db.child_ix_of(id) == 0
            && (db.is_user_node(id)
                || (db.same_name_monomorphic_ctor(name) && !db.is_type_expr_node(id))
                || inlined_value_construct)
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
    // 3b′. A TOP-LEVEL `(module NAME …)` declaration — a sibling of the top-level defs, `(do (module m …)
    // (def (main) …) (export main))`. It binds `m` PROGRAM-WIDE, resolved EXACTLY like a top-level
    // def/type/effect (a `Ref` to its synthesized export record), so a reference from any top-level def's
    // body resolves and `(. m field)` is ordinary member access. A do-LOCAL module (nested in a def body's
    // `(do …)`) is lexically scoped and already bound by step 1's scope walk (`do_local_binds`); this step
    // consults ONLY top-level modules (`top_level_module_by_name`), so a local module is not leaked
    // program-wide. Placed after defs/types/effects — a same-named binder/def/type/effect shadows a module
    // name — and before the prelude, matching the other declaration steps (`prelude-and-resolution.md`
    // §Name Resolution Is One Ordered Lookup; nothing is privileged by name).
    //= spec/capabilities/core-semantics.md#a-module-binds-its-name-in-its-enclosing-scope
    //# Evaluating a module MUST bind the module's declared name in the enclosing scope to the record of the module's exports, so that a module is named by its declaration without a separate binding form.
    if let Some(value) = db.top_level_module_by_name(name) {
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → top-level module decl");
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
    // FILE-SCOPED for a linked package (the ctor analogue of the type-name step): a bare variant name
    // resolves against the reference's own file (own type decls + imported types), never a sibling's
    // private ctor. `Some(Err(()))` — file known, ctor not visible — falls through so a same-named
    // PRELUDE ctor (`Some`/`None`/`Ok`/`Err`) can still apply. `None` (indeterminate node) falls to the
    // flat path.
    let scoped_ctor = if db.is_linked_package() {
        db.file_scoped_variant_ctor(id, name)
    } else {
        None
    };
    if let Some(Ok(value)) = scoped_ctor {
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → file-scoped user sum variant ctor");
        return Resolved::Ref { value };
    } else if scoped_ctor.is_none()
        && let Some(value) = db.variant_ctor_by_name(name)
    {
        // Reached only when NOT file-scoped; a linked `Some(Err(()))` (ctor not visible in this file)
        // falls through so a same-named PRELUDE ctor can still apply.
        trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → user sum variant ctor");
        return Resolved::Ref { value };
    }
    // (The old cross-component `(extern …)` op-resolution step was REMOVED in U4: cross-component interop
    // is now unified with EFFECTS — a peer contract is an `(effect …)` bound to a peer via `(bind …)`, and
    // an escaping op is an ordinary perform that lowers to a `Core::HostCall` routed to the peer envelope.
    // There is no separate extern-op name to resolve.)
    // 3d. A bare variant whose name COLLIDES with a prelude TYPE/MODULE name (`Int`/`List`/`Name`/…) —
    // the ones step 3c's `variant_ctor_by_name` (and the file-scoped bare map) DELIBERATELY omits (the
    // `9f326a2d` skip) so bare `Int` keeps resolving to the width constructor everywhere it means the
    // width TYPE (a `(Int W)` annotation reduction, `Int64`'s synthesized `(Int 64)`). But in
    // application-HEAD position on a genuine USER node a bare `(Int 42)` is a value CONSTRUCT of the local
    // variant, not a width-type reduction — so the user's variant SHADOWS the colliding prelude name here
    // (operator ruling 2026-07-15: a program-defined name shadows the built-in alias in construct position
    // too, consistent with binding-is-lexical and the match-pattern remap `85faf395`). The `child_ix == 0`
    // + `is_user_node` gate is the SAME head-position/user-node discriminator the same-name-newtype ctor
    // step uses: it fires on a real source `(Int 42)` construct but NOT on a synthesized `(Int 64)` the
    // width machinery builds (a non-user node) nor on a non-head `Int` used as the width TYPE. Placed after
    // the ordinary variant step + before the prelude, so a NON-colliding variant already resolved above and
    // only a colliding one (skipped from the bare index) reaches here. Generic — no name special-case; the
    // set of shadowable names is exactly the user's colliding variant declarations.
    //
    // EXCLUDE a TYPE-EXPRESSION head: `(List Ast)` in a variant payload / annotation `(: x (List Int64))`
    // heads a list on a user node too, but there `List` is the TYPE constructor, not a value construct of a
    // `List` variant — diverting it would turn a self-referential AST sum's payload into a bogus variant
    // application (CDZ0201). A bare type ATOM (`Int64`) is already spared by the head-position gate; only a
    // type-expression APPLICATION head needs this. `is_type_expr_node` is the load-time subtree marker.
    if db.child_ix_of(id) == 0 && db.is_user_node(id) && !db.is_type_expr_node(id) {
        // FILE-SCOPED for a linked package: consult the QUALIFIED ctor surface (which, unlike the bare
        // map, retains a prelude-named ctor) confined to this reference's own file, so the shadowing is
        // scoped exactly like every other declaration — a sibling file's colliding variant does not leak.
        // A single-file / indeterminate-node compile falls through to the flat companion index.
        let scoped = if db.is_linked_package() {
            db.file_scoped_variant_ctor_qualified(id, name)
        } else {
            None
        };
        if let Some(Ok(value)) = scoped {
            trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → file-scoped prelude-colliding variant ctor (construct-position shadow)");
            return Resolved::Ref { value };
        } else if scoped.is_none()
            && let Some(value) = db.prelude_colliding_variant_ctor(name)
        {
            trace!(target: "rcdzc::resolve", node = id.0, %name, bound_to = value.0, "name → prelude-colliding variant ctor (construct-position shadow)");
            return Resolved::Ref { value };
        }
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
        // A `N`-suffixed literal (`→ BigInt`) whose body is written in FLOAT FORM — a decimal point or an
        // exponent, `0.5N` / `2.0N` / `1e3N` — is the common suffix slip: `N` means BigInt, an UNBOUNDED
        // INTEGER, which is spelled as a plain integer (no `.`/exponent), so a float-form body is not a
        // BigInt literal. The generic "malformed numeric literal" reads as a lexer complaint; name the
        // actual cause and the fix (a decimal is a `Rational`, so the `R` suffix — which admits a float
        // body — is what fits). Detected self-contained (no cross-crate literal re-parse): a digit-led
        // token ending in `N` whose body contains a `.` or an exponent marker. `R` admits both integer and
        // float bodies, so it never reaches here; a genuinely garbled token (`0xGG`, `12abc`) keeps the
        // generic message.
        if let Some(body) = name.strip_suffix('N')
            && body.starts_with(|c: char| c.is_ascii_digit())
            && (body.contains('.') || body.contains('e') || body.contains('E'))
        {
            // The message names the exact repair (`{body}R`) — swapping the `N` suffix for `R` on the same
            // digit body — so carry it as a structural replace fix, not prose only (`spec/capabilities/
            // diagnostics.md` §A Diagnostic Carries A Route To A Fix). Heuristic: `R` is the likeliest
            // intent for a decimal value, but the author might instead have meant to drop the fraction for
            // a true BigInt — the swap clears the malformed token either way, and the choice is theirs.
            return Resolved::Poison(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "the `N` suffix means BigInt (an unbounded integer), which is written as a plain \
                         integer, but `{name}` is in decimal/float form — for a decimal value use the `R` \
                         (Rational) suffix: `{body}R`"
                    ),
                )
                .at(id)
                .with_fix(crate::diag::Fix::replace_heuristic(id, format!("{body}R"))),
            );
        }
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!("malformed numeric literal `{name}`"),
        ));
    }
    // The WILDCARD `_` used as a VALUE — `(+ _ 1)`, `(g _)`, a bare `_` def body. `_` is a binding-position
    // wildcard (it discards the bound value in a pattern — `((tuple _ b) …)`, `(let ((_ e)) …)`); it names
    // no value, so a reference to it is not an ordinary unbound name (a "did you mean?" typo suggestion is
    // nonsense for it) but a category misuse. Name that specifically — it reads far better than "unbound
    // name `_`" and points at the two legitimate uses. (A `_`-LED binder like `_x` is an ordinary name — a
    // silenced binder — and never reaches here; only the bare `_` wildcard does.)
    if name == "_" {
        trace!(target: "rcdzc::resolve", node = id.0, "wildcard `_` used as a value (CDZ0201)");
        return Resolved::Poison(
            Reject::coded(
                Code::Malformed,
                "`_` is a wildcard, not a value — it may appear only in a binding position (a pattern \
                 element like `(tuple _ x)`, or a discarded `let` binder), where it discards the value; \
                 use a named binder to refer to a value",
            )
            .at(id),
        );
    }
    // `?` used as the HEAD of a form — `(? e)`. The diagnostics for the fallible short-circuit operator
    // consistently call it "`?`/`try`", and `?` is the sigil many languages spell it with, so an author
    // reaches for `(? e)`; but the s-expr surface head is the KEYWORD `try` (`(try e)`), and `?` is not a
    // bound name, so it resolves unbound — a misleading "unbound name `?`" (a did-you-mean scan over scope
    // is nonsense for a sigil). Name the real spelling + carry a VERIFIED head-rewrite `?` → `try` (the
    // operand is preserved; the rewrite is the exact form the author meant). Only in HEAD position — a bare
    // `?` elsewhere keeps the ordinary unbound path. `try` is a grammar keyword, so this never shadows a
    // user name.
    if name == "?"
        && db.parent_of(id).is_some_and(
            |p| matches!(db.ast.get(p), Struct::List(kids) if kids.first() == Some(&id)),
        )
    {
        trace!(target: "rcdzc::resolve", node = id.0, "`?` head → the try operator is spelled `(try e)`");
        return Resolved::Poison(
            Reject::coded(
                Code::Unbound,
                "the fallible short-circuit operator is spelled `(try <expression>)` in this surface, \
                 not `(? …)` — write `try` as the head",
            )
            .at(id)
            .with_fix(crate::diag::Fix::replace_verified(id, "try", "replace `?` with `try`")),
        );
    }
    // `..` used as a VALUE / form HEAD — `(.. xs)`, `(g ..)`. `..` is the REST/SPREAD marker of a
    // collection PATTERN (`(list a .. rest)`, `(map (k v) .. rest)`); a real pattern rest is consumed by
    // the list/map pattern parser (which scans for `as_name(e) == Some("..")`) and NEVER reaches here as a
    // value reference. So a `..` that DOES reach `resolve_name` is a category misuse — the pattern-only
    // sigil written where a value/head belongs. The generic path gave "unbound name `..`" (and, in head
    // position, a misleading "did you mean `.`?" — `..` is distance-1 from member-access `.`, but a rest
    // marker is not a mistyped `.`). Name the real role instead. NO fix: a rest marker has no value
    // rewrite, and the `.`-rename is a wrong guess. (Fires on ANY bare `..`, like the `_` branch — `..`
    // never denotes a value; a `..`-LED name is not a thing.) The uppercase/`?`/`_` sibling of the
    // sigil-in-value-position family.
    if name == ".." {
        trace!(target: "rcdzc::resolve", node = id.0, "`..` rest marker used as a value (CDZ0201)");
        return Resolved::Poison(
            Reject::coded(
                Code::Malformed,
                "`..` is a rest/spread marker, valid only inside a collection PATTERN — a `(list …)`, \
                 `(map …)`, `(tuple …)`, `(record …)`, or `(set …)` pattern (e.g. `(list a .. rest)`, \
                 which binds the leading elements and the tail) — it is not a value or a form head here",
            )
            .at(id),
        );
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
pub(crate) fn nearest_unbound_suggestion(db: &mut Db, id: StructId, name: &str) -> Option<String> {
    nearest_name_suggestion(db, id, name)
}

/// When the unbound name at `id` is the HEAD of a form `(id …)` AND its nearest-name suggestion is a
/// GRAMMAR KEYWORD (`match`/`let`/`if`/`and`/… — a misspelled control/binding form, e.g. `(mtch …)` for
/// `match`, `(le …)` for `let`), return that ENCLOSING form node. `None` otherwise (a non-head typo, a
/// head whose best match is an ordinary value/def, or a head with no close match). Used by
/// `compile::collect_faults` to SUPPRESS the CASCADE a misspelled keyword triggers: the whole form is
/// (mis)read as an APPLICATION, so its arms/bindings fault too (`(mtch n (0 1) …)` → "cannot apply Int64"
/// on the arm `(0 1)`, "unbound `_`" on the wildcard) — all CONSEQUENT on the head typo, not independent
/// problems. Keying on the suggestion being a GRAMMAR keyword (not any name) keeps the suppression to the
/// misspelled-form case: an ordinary misspelled FUNCTION head `(helpr x)` has a real callee typo, and its
/// arguments are genuine sub-expressions whose own faults ARE independent — so they are never suppressed.
pub(crate) fn unbound_head_suggests_grammar_keyword(db: &mut Db, id: StructId) -> Option<StructId> {
    let parent = db.parent_of(id)?;
    // `id` must be the FIRST child (the head) of its parent list.
    if !matches!(db.ast.get(parent), Struct::List(kids) if kids.first() == Some(&id)) {
        return None;
    }
    let name = db.ast.as_name(id)?.to_string();
    let candidate = nearest_name_suggestion(db, id, &name)?;
    is_grammar_head(&candidate).then_some(parent)
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
fn nearest_name_suggestion(db: &mut Db, id: StructId, name: &str) -> Option<String> {
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
    // HEAD position — `id` is the FIRST child of a List `(id …)`, so the author may have meant a GRAMMAR
    // KEYWORD (`(mtch …)` for `match`, `(iff …)` for `if`, `(le …)` for `let`). A correctly-spelled
    // grammar head is dispatched structurally (never reaches here); only a MISSPELLED one falls through to
    // "unbound name", where — without the keywords in the pool — it got no suggestion and often cascaded
    // (the mis-parsed body then faulting too). A head is ALSO a value position (a call `(helper 5)`), so
    // the value tiers still apply; the keywords are ADDED candidates, not a replacement. Not a member
    // operand / type expr (those are non-head positions).
    let head_position = !non_value_position
        && matches!(parent, Some(p)
            if matches!(db.ast.get(p), Struct::List(kids) if kids.first() == Some(&id)));
    // Tier 1 — lexical scope: every binder visible where the reference sits. This is the ONLY per-NODE
    // tier (a param/`let`/pattern binder near THIS reference), and it is small, so it is collected fresh.
    let mut tier1: Vec<String> = Vec::new();
    if !non_value_position {
        for (n, _occ) in visible_bindings(db, id) {
            tier1.push(n);
        }
    }
    // The PROGRAM-WIDE winner (defs / variants / effects / types / prelude) is MEMOIZED per (name, class):
    // its edit-distance scan over the whole pool is O(pool), and N call sites of the SAME missing name
    // re-ran the identical scan → O(N²) (a forgotten import / renamed helper referenced from N sites).
    // Combining the memoized pool winner with the tiny per-node tier-1 scan reproduces the global result
    // EXACTLY: `nearest` prefers lower edit distance, ties broken lexicographically, so re-running it over
    // `tier1 ∪ {pool_winner}` picks the same name `tier1 ∪ pool` would (a pool candidate that lost to the
    // pool winner — farther, or equal-distance-but-lexicographically-larger — could never have beaten it
    // against tier1 either).
    let pool_winner = pool_suggest_winner(db, member_operand, type_expr, name);
    // In head position, the GRAMMAR keywords join the candidate set — a small FIXED const, so scanned
    // inline (no memoized pool). `HANDLE_INTERNAL` is a synthesized desugar token, never something an
    // author types, so it is excluded from the user-facing suggestions.
    let grammar_candidates: &[&str] = if head_position { GRAMMAR } else { &[] };
    crate::diag::suggest::nearest(
        name,
        tier1
            .iter()
            .map(String::as_str)
            .chain(pool_winner.as_deref())
            .chain(
                grammar_candidates
                    .iter()
                    .copied()
                    .filter(|k| *k != crate::effects::HANDLE_INTERNAL),
            ),
    )
}

/// The program-wide typo-suggestion winner (the nearest name in the position-class pool to `name`),
/// MEMOIZED per `(name, class)` in `db.suggest_pool_winner` — so N unbound references to the SAME missing
/// name share one O(pool) edit-distance scan instead of re-running it each (the O(N²) fix). Builds the
/// class pool once (`program_suggest_pool`, itself cached), scans it via `suggest::nearest`, and caches
/// the winner. A pure function of the program + query.
fn pool_suggest_winner(
    db: &mut Db,
    member_operand: bool,
    type_expr: bool,
    name: &str,
) -> Option<String> {
    let class = if member_operand {
        1u8
    } else if type_expr {
        2
    } else {
        0
    };
    let key = (name.to_string(), class);
    if let Some(hit) = db.suggest_pool_winner.get(&key) {
        return hit.clone();
    }
    let pool = program_suggest_pool(db, member_operand, type_expr);
    let winner = crate::diag::suggest::nearest(name, pool.iter().map(String::as_str));
    db.suggest_pool_winner.insert(key, winner.clone());
    winner
}

/// The PROGRAM-WIDE typo-suggestion candidate names for a position class (the def / variant / effect /
/// type / prelude tiers of [`nearest_name_suggestion`], everything EXCEPT the per-node lexical tier) —
/// built once per class and CACHED in `db.suggest_pool`, so N unbound occurrences do not each re-clone
/// every def name (the O(N²) that "N call sites of one missing name" hit). A pure function of the program:
/// the same three position classes `nearest_name_suggestion` distinguishes (value / member-operand /
/// type-expression) select which tiers appear, exactly as the inline build did — the produced set is
/// byte-identical, only its construction is amortized.
fn program_suggest_pool(
    db: &mut Db,
    member_operand: bool,
    type_expr: bool,
) -> std::rc::Rc<Vec<String>> {
    let class = if member_operand {
        1
    } else if type_expr {
        2
    } else {
        0
    };
    if let Some(pool) = &db.suggest_pool[class] {
        return pool.clone();
    }
    let non_value_position = member_operand || type_expr;
    let mut pool: Vec<String> = Vec::new();
    if !non_value_position {
        // Tier 2 — this module's top-level value definitions (value position only).
        for d in &db.defs {
            pool.push(d.name.clone());
        }
        // Tier 2b — the boolean LITERALS `true`/`false` (value position only). They are lexer literals
        // (`Leaf::Bool`), not bound names, so they are NOT in scope/defs/prelude — yet a mis-cased
        // `True`/`False` (the cross-language habit) reads as an unbound NAME, and its one-shot fix is the
        // lowercase literal (which re-lexes as `Leaf::Bool`). Offering them as candidates makes `True` →
        // "did you mean `true`?" (edit distance 1, within the cutoff), the same did-you-mean an unbound
        // name gets — a literal is a valid replacement here exactly as a name is. (`TRUE` is distance 4,
        // beyond the cutoff, so only the common single-case-slip is suggested — no baseless guess.)
        pool.push("true".to_string());
        pool.push("false".to_string());
    }
    // Tier 3 — `(type …)` names (a type name fits BOTH a member operand `Int64.max` AND a type expr `(: x
    // Int64)`) + their variant CONSTRUCTORS (a value, so kept ONLY in value position) — and `(effect …)`
    // names (member-accessible, so a member-operand candidate; NOT a type, so excluded from a type expr).
    for t in &db.type_decls {
        pool.push(t.name.clone());
        if !non_value_position {
            for v in &t.variants {
                pool.push(v.name.clone());
            }
        }
    }
    if !type_expr {
        for e in &db.effect_decls {
            pool.push(e.name.clone());
        }
    }
    // Tier 4 — the prelude's built-in names. In a NON-VALUE position drop the prelude's VARIANT
    // CONSTRUCTORS (`None`/`Some`/`Ok`/`Err`): a variant is a value, member-inaccessible AND not a type, so
    // suggesting `Nope`→`None` in `(. Nope op)` / `(: x Nope)` position would fail the one-shot rule. The
    // discriminant is the prelude BINDING's own shape — `variant_disc_of` on the entry's value node
    // (a `(meta variant)` channel) — NOT a name-set membership: a name can be BOTH a member-accessible
    // MODULE and some sum's variant (`List` is the collection module AND the `Ast.List` node kind), and a
    // name-collision filter wrongly dropped such a module from a member-operand pool (so `Lst.len` could
    // not suggest `List`). Testing the prelude value directly keeps a module that merely shares a name with
    // a variant, and still drops a genuine prelude variant constructor.
    let prelude_keys: Vec<(String, StructId)> =
        db.prelude.iter().map(|(k, v)| (k.clone(), *v)).collect();
    for (key, value) in prelude_keys {
        if non_value_position && crate::eval::variant_disc_of(db, value).is_some() {
            continue;
        }
        pool.push(key);
    }
    let rc = std::rc::Rc::new(pool);
    db.suggest_pool[class] = Some(rc.clone());
    rc
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
                // A match ARM `(pattern body)` ascended from `body` → the names its PATTERN binds. A bare
                // binder (`n`) binds itself; a COMPOUND pattern — `(Some p)`, `(tuple a b)`, `(list x ..
                // rest)` — binds every bare-name leaf inside it. Without the compound case a typo of an
                // element/rest binder (`rst` for `rest`) had NO in-scope candidate and mis-suggested a
                // far prelude name (`Ast`); collecting the pattern's binders makes it suggest `rest`. The
                // pool is generous (a ctor name inside the pattern may slip in), which is harmless for a
                // did-you-mean — a wrong candidate only surfaces if it is the NEAREST, and a pattern's
                // own names are exactly what is in scope in the arm body.
                if let Struct::List(arm) = db.ast.get(form)
                    && arm.len() == 2
                    && Some(from) == arm.get(1).copied()
                    && db
                        .parent_of(form)
                        .and_then(|p| db.ast.as_form(p, "match"))
                        .is_some()
                {
                    for (n, occ) in arm_pattern_binders(db, arm[0]) {
                        push(&n, occ, &mut out);
                    }
                }
            }
        }
        from = form;
        cursor = db.parent_of(form);
    }
    out
}

/// The `(name, occurrence)` binders a match-arm PATTERN introduces — used to seed `visible_bindings`'s
/// did-you-mean candidate pool for a reference in the arm body. A bare name binds itself; a COMPOUND
/// pattern (`(Some p)`, `(tuple a b)`, `(list x .. rest)`, `(map (k v) .. rest)`) binds every bare-name
/// LEAF inside it. Deliberately SYNTACTIC and immutable (`&Db`) — it does NOT resolve ctor-vs-binder (a
/// `&mut Db` operation `collect_pattern_binders` in `lower` does that for linearity), so a nullary-ctor
/// name inside the pattern may slip into the pool; that is harmless for a suggestion (a candidate only
/// surfaces if it is the NEAREST to the typo, and the arm's own names are exactly what is in scope). The
/// separators `_` (wildcard) and `..` (rest marker) bind nothing and are skipped. Bounded recursion over
/// the pattern tree; a pattern is shallow, so no depth guard is needed.
pub(crate) fn arm_pattern_binders(db: &Db, pat: StructId) -> Vec<(String, StructId)> {
    let mut out: Vec<(String, StructId)> = Vec::new();
    collect_arm_binder_leaves(db, pat, &mut out);
    out
}

fn collect_arm_binder_leaves(db: &Db, pat: StructId, out: &mut Vec<(String, StructId)>) {
    match db.ast.get(pat) {
        // A bare name is a binder unless it is a separator (`_`/`..`). A literal atom has no name, so
        // `as_name` yields `None` and it is skipped.
        Struct::Atom(_) => {
            if let Some(n) = db.ast.as_name(pat)
                && n != "_"
                && n != ".."
            {
                out.push((n.to_string(), pat));
            }
        }
        // A `.`-MEMBER form `(. Sum V)` — a NULLARY-variant ctor reference used as a whole pattern (`C.R`).
        // Its segments after `.` are the TYPE and VARIANT names, NOT binders — so it binds NOTHING. Without
        // this arm the generic `skip(1)` recursion below collected the first segment (`C`) as a spurious
        // binder → a bogus CDZ0306 "unused binding `C`" (with a nonsense `_C` rename) on every `(match c
        // (C.R …) …)` arm. (An APPLIED ctor `(C.V arg…)` still recurses its ARGS below — its head is a
        // separate node, correctly skipped; only the bare-member-as-whole-pattern case is the ctor here.)
        Struct::List(children)
            if children
                .first()
                .is_some_and(|&h| db.ast.as_name(h) == Some(".")) =>
        {
            // A member form binds nothing.
        }
        // A `(bin <seg>…)` binary pattern — each segment is `(<kind> <slot> [modifier|size]…)`. ONLY the
        // SLOT (`seg[1]`) introduces a binder; the trailing atoms are a byte-order MODIFIER (`le` on an
        // int segment), a bit-field WIDTH (`k`, a constant), or a dependent-SIZE operand (`n` on
        // `bytes`/`utf8`, a reference to an already-bound size, not a new binding) — none of which the
        // unused-binding lint should treat as a match binder. The generic `skip(1)` recursion below would
        // walk EVERY post-head atom and collect `le` (adv-59: a false CDZ0306 "unused match binding `le`",
        // whose suggested `_le` fix HARD-ERRORS since the segment stops parsing as a modifier). So descend
        // each segment's slot only. (A `(bin)` with no segments binds nothing.)
        Struct::List(children)
            if children
                .first()
                .is_some_and(|&h| db.ast.as_name(h) == Some("bin")) =>
        {
            for &seg in children.iter().skip(1) {
                // A segment is itself `(<kind> <slot> …)`; the slot is its second child. Recurse into the
                // slot (a bare binder, or a nested pattern the slot may carry) — never the kind head or the
                // trailing modifier/width/size atoms.
                if let Struct::List(parts) = db.ast.get(seg)
                    && let Some(&slot) = parts.get(1)
                {
                    collect_arm_binder_leaves(db, slot, out);
                }
            }
        }
        // A record-pattern FIELD `(= field sub-pattern)` (path B — same form as a value-record field):
        // the field NAME binds nothing (it names the record slot, not a binder), only the sub-pattern
        // (child 2) does. Without this arm the generic recursion below would collect the field name as a
        // spurious binder (the `=` head is skipped, then BOTH `field` and the sub-pattern are walked).
        Struct::List(children)
            if children.len() == 3 && db.ast.as_name(children[0]) == Some("=") =>
        {
            collect_arm_binder_leaves(db, children[2], out);
        }
        // A SET pattern `#set(e… .. rest)` — its named ELEMENTS are ordinary value expressions (the set twin
        // of a map KEY), NOT binders, so they bind NOTHING; only the REST binder (after `..`) is a binder.
        // Without this the generic recursion below collected every element as a spurious binder → a false
        // CDZ0306 "unused match binding" on an IN-SCOPE name element (`#set(k)`, `k` a param used only for
        // its membership value), whose bogus `_k` rename would also break the membership test.
        Struct::List(_)
            if db
                .ast
                .compound_form_of(pat, crate::ast::CompoundCtor::Set)
                .is_some() =>
        {
            let tail = db
                .ast
                .compound_form_of(pat, crate::ast::CompoundCtor::Set)
                .expect("set form (checked in guard)");
            if let Some((i, _op, _trailing)) = db.ast.rest_marker(tail) {
                // Collect ONLY the rest region (after `..`): the flat `.. rest` binder, or a wrapped
                // `(.. rest)` (recursed). The elements before `..` bind nothing.
                for &e in &tail[i..] {
                    collect_arm_binder_leaves(db, e, out);
                }
            }
        }
        // A compound pattern `(head arg…)` — skip the HEAD (a ctor / `list`/`tuple`/`map`/`record` alias
        // / `guard`), recurse the arguments. A `(map (k v) …)` entry is itself a compound, so the
        // recursion reaches its `k`/`v` leaves naturally. A `(record (= f p) …)` field is handled by the
        // arm above; a legacy `(record (f p))` pair still recurses here (head `f` skipped, `p` collected).
        Struct::List(children) => {
            for &arg in children.iter().skip(1) {
                collect_arm_binder_leaves(db, arg, out);
            }
        }
    }
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
    #[cfg(test)]
    crate::db::BINDER_IN_CALLS.with(|c| c.set(c.get() + 1));
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
            // The body-match is normally `body_occ` by identity. But a lowering-time desugar
            // (`desugar_refutable_literal_list_elements`) REPLACES the body match with a freshly-built
            // `rewritten` match reparented into the body SLOT — same parent + child position, DIFFERENT
            // node id — without updating the `let`'s recorded `body_occ` (the arena list is immutable). So
            // also accept `from` when it occupies the body's child POSITION under this `let` (its parent is
            // `form` and its child index equals `body_occ`'s). Without this, a guard-cond reference to a
            // `let` binding in such a rewritten body ascends to the `let` but fails the identity check →
            // spurious CDZ0101 unbound (the inlined guarded-literal-list bug: the β-copy re-parents the
            // `let`/`match`, the desugar re-runs on the copy, and the copied `let`'s `body_occ` still points
            // at the pre-desugar copied match). Position-based, so it never widens a genuine non-body child.
            let from_is_body = Some(from) == body_occ
                || (body_occ.is_some()
                    && db.parent_of(from) == Some(form)
                    && body_occ.is_some_and(|b| db.child_ix_of(from) == db.child_ix_of(b)));
            if from_is_body {
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
    // Case 4b: `form` is a `(def (NAME param…) body)`'s SIGNATURE LIST, ascended from a parameter `from`
    // (a later param's annotation). An EARLIER type-valued parameter is visible in a later parameter's
    // annotation — `(def (unbox (: t Type) (: b (Box t))) …)`, where `(Box t)` reads the earlier `t`. This
    // is the signature's IN-ORDER scope, the type-valued-parameter analogue of a `let`'s in-order bindings
    // (Case 2): a param sees the params written before it, not itself or later ones. `NAME` at position 0
    // is the def name, never a parameter, so it is excluded by the window (`from` is a param, always after
    // NAME). Only reached for a def SIGNATURE list (`def_sig_list_of` confirms the parent is a `def` and
    // `form` is its first tail element) — a def body ascends through Case 4 above, not here.
    if def_sig_list_of(db, form).is_some() {
        return param_binder_before(db, form, name, from);
    }
    // FAST-REJECT for the O(depth)-per-reference match-arm pole: everything below (Cases 5..6mg) fires
    // only when `form` is a MATCH ARM, and every one binds a name written into the arm's PATTERN. If the
    // load-time index knows `form` is an arm whose pattern has no `name` atom, no arm case can bind it —
    // skip the ~20-case cascade and return `None` in O(1). A reference to a GLOBAL/prelude/ctor name
    // (`Some`, `None`, `+`) is bound by no arm, so in a deeply-nested match it ascends O(depth) arms; this
    // makes each such hop O(1) instead of running the full cascade (the measured resolve pole). SAFE: the
    // index over-approximates the arm's bound names, so a name the arm CAN bind is never fast-rejected;
    // an arm absent from the index (a synthesized β-copy) returns `false` here and takes the cascade.
    if db.arm_cannot_bind(form, name) {
        return None;
    }
    // Count entries into the arm cascade below — the noise-free signal that the head-excluding fast-reject
    // above keeps cascade entries O(N) (not O(N²)) on a deeply-nested match (see `ARM_CASCADE_ENTRIES`).
    #[cfg(test)]
    crate::db::ARM_CASCADE_ENTRIES.with(|c| c.set(c.get() + 1));
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
    // Case 6l: `form` is a MATCH ARM `((list p… …) body)` (ascended from `body`) whose list pattern binds
    // `name` at a LEADING element position — possibly NESTED inside that element's own sub-pattern. Reading
    // leading element `i` of the (list) scrutinee is the SAME `Elem(i)` access a tuple-payload binder uses,
    // so it reuses `SumPayload`; a nested `(tuple a b)` / `(Mk n)` element extends the path with the
    // sub-pattern's own `Elem`/`Payload` steps (element positions compose, §145). Its type is walked from
    // the list's element type; a constant scrutinee folds the leaf (`fold_sum_path`). Scoped to this arm.
    // (A REST binder binds a SUBLIST — Case 6r, `list_pattern_rest_binds`.)
    if let Some((scrutinee, steps, heads)) = list_pattern_element_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            steps: steps.into(),
            heads: heads.into(),
        });
    }
    // Case 6r: `form` is a MATCH ARM `((list p… .. rest) body)` (ascended from `body`) whose REST pattern
    // binds `name` as the rest binder (the name after `..`). Unlike a leading element (one value), the
    // rest binds the TAIL SUBLIST from index `lead` onward — a `SumPayload` with a bare `[RestFrom(lead)]`
    // path (no head). Its type is `(List elem)` (same as the scrutinee); a constant scrutinee folds the
    // tail (`fold_sum_path`'s `RestFrom`/`ListNew` arm), a runtime list reads `vec-split(list, lead).right`.
    if let Some((scrutinee, lead)) = list_pattern_rest_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            steps: vec![crate::core::PathStep::RestFrom(lead)].into(),
            heads: vec![].into(),
        });
    }
    // Case 6mr: `form` is a MATCH ARM whose pattern is a `(list p… .. <nested-pattern>)` — the rest slot
    // after `..` holds a NESTED pattern, not a bare name/`_`, binding `name` (an inner binder like `b`/`r`
    // in `(list a .. (list b .. r))`). RULED (v-inference 2026-08-02): a list rest binder admits only a
    // name or wildcard (core-semantics.md:149 grants nested patterns to ELEMENT positions only; :135 a
    // binding position holds an irrefutable pattern, and a nested list pattern is refutable on empty rest —
    // the same name-only rule the map rest binder has). So the REJECT is correct, but WITHOUT this the inner
    // binder falls through to scoping → a MISLEADING CDZ0101 "unbound name `b`" that masks the real
    // rest-shape fault. Resolve it to a coded SHAPE decline naming the rest form, SUPPRESSING the unbound
    // cascade — the list twin of Case Mmr (malformed map rest). ANCHOR at the offending `(list …)` PATTERN
    // node (same node the list matcher's own shape reject lands on — `lower.rs`), so same-node dedup collapses
    // the two into ONE primary diagnostic (the in-pattern binder occurrences already inert via
    // `is_list_pattern_element_occurrence`, so only these body references reach here).
    if let Some(list_pat) = match_arm_nested_list_rest_binds(db, form, from, name) {
        return Some(Resolved::Poison(
            Reject::coded(
                crate::diag::Code::Malformed,
                crate::diag::LIST_REST_BINDER_NAME_ONLY,
            )
            .at(list_pat),
        ));
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
    // Case 6lg: `form` is a GUARD `(guard (list p… [.. rest]) <cond>)`, ascended from `<cond>` → a LEADING
    // element binder or the REST binder of the list pattern is in scope in the guard cond (`(guard (list x
    // .. rest) (> x 0))` — the guard reads `x`). Like Case 6g but the binder is in a LIST pattern, so it
    // resolves to a `SumPayload` at an `Elem`/`RestFrom` path. Caught here for the same reason as 6g (at
    // the arm, `from` is the guard wrapper, not the cond).
    if let Some((scrutinee, steps, heads)) = guard_cond_list_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            steps: steps.into(),
            heads: heads.into(),
        });
    }
    // Case 6tg: `form` is a GUARD `(guard (tuple p0 p1 …) <cond>)`, ascended from `<cond>` → an element
    // binder of the tuple pattern (possibly nested) is in scope in the guard cond (`(guard (tuple a b) (> (+
    // a b) 5))` — the guard reads `a`/`b`). The TUPLE analogue of Case 6lg: `find_binder_in_tuple` descends
    // each element (recursing into nested tuple/list/ctor sub-patterns), so the binder resolves to a
    // `SumPayload` at its `Elem(i)`/… path. Without this, a guard on a tuple arm reported CDZ0101 unbound for
    // its element binders (`find_binder_in_pattern`, which `guard_cond_variant_binds` uses, EXCLUDES the
    // `tuple` head). Caught here for the same reason as 6g/6lg (at the arm, `from` is the guard wrapper).
    if let Some((scrutinee, steps, heads)) = guard_cond_tuple_binds(db, form, from, name) {
        return Some(Resolved::SumPayload {
            scrutinee,
            steps: steps.into(),
            heads: heads.into(),
        });
    }
    // Case 6recg: `form` is a GUARD `(guard (record (x a) …) <cond>)`, ascended from `<cond>` → a field
    // binder of the record pattern is in scope in the guard cond (`(guard (record (x a)) (> a 0))` — the
    // guard reads `a`). The record analogue of Case 6tg: a bare-binder field resolves to a `Member`
    // projection of the scrutinee at that field (the same form Case 6rec gives the body reference). Caught
    // here for the same reason as 6g/6lg/6tg (at the arm, `from` is the guard wrapper, not the cond).
    if let Some((scrutinee, key)) = guard_cond_record_binds(db, form, from, name) {
        return Some(Resolved::Member {
            operand: scrutinee,
            key,
        });
    }
    // Case 6recng: `form` is a GUARD `(guard <compound with a NESTED (record …)> <cond>)` ascended from the
    // cond → a bare-binder field of a record NESTED in the guard's variant/tuple/list pattern is in scope in
    // the guard cond (`(guard (Some (record (= x a))) (> a 5))` — the guard reads `a`). The guard-cond twin of
    // Case 6rec-nested: the binder resolves to the SAME `RecordField` the arm BODY reference gets, so the
    // guard reads the nested field off the scrutinee exactly as the body would. Caught here (form=guard,
    // from=cond) because at the arm `from` is the guard wrapper, not the cond, so
    // `match_arm_nested_record_binds_path` cannot fire for a guard-cond reference (its `from == guard_cond`
    // check misses). Without this, a guard reading a nested-record binder was a spurious CDZ0101 while the
    // body reading the same binder compiled (breaker's s3).
    if let Some((scrutinee, path, key, sub_path, heads)) =
        guard_cond_nested_record_binds_path(db, form, from, name)
    {
        return Some(Resolved::RecordField {
            scrutinee,
            path: path.into(),
            key,
            sub_path: sub_path.into(),
            heads: heads.into(),
        });
    }
    // Case 6bg: `form` is a GUARD `(guard (bin <seg>…) <cond>)`, ascended from `<cond>` → a SEGMENT binder of
    // the bin pattern is in scope in the guard cond (`(guard (bin (u8 n)) (> n 5))` — the guard reads `n`).
    // The binary analogue of Case 6lg/6tg/6recg: the binder resolves to the SAME `BinField` Case B gives the
    // arm BODY, so the guard decodes the segment off the scrutinee exactly as the body would. Caught here for
    // the same reason as the other guard-cond cases (at the arm, `from` is the guard wrapper, not the cond).
    if let Some((scrutinee, segs, seg_index)) = guard_cond_bin_binds(db, form, from, name) {
        return Some(Resolved::BinField {
            scrutinee,
            segs: segs.into(),
            seg_index,
        });
    }
    // Case 6mg: `form` is a GUARD `(guard (map (k v) … .. rest) <cond>)`, ascended from `<cond>` → a VALUE
    // binder `v` (at key `k`) or the REST binder is in scope in the guard cond (`(guard (map (1 v)) (> v 5))`
    // — the guard reads `v`). The MAP analogue of Case 6lg/6tg/6recg/6bg: the binder resolves to the SAME
    // `MapField` Case M gives the arm BODY, so the guard reads the map value/rest off the scrutinee exactly as
    // the body would (the runtime map desugar reuses both the body AND the guard verbatim, so both must carry
    // their `MapField` resolution BEFORE the desugar runs). Caught here for the same reason as the other
    // guard-cond cases (at the arm, `from` is the guard wrapper, not the cond).
    if let Some((scrutinee, key, named, value_steps, value_heads)) =
        guard_cond_map_binds(db, form, from, name)
    {
        return Some(Resolved::MapField {
            scrutinee,
            path: std::rc::Rc::from(Vec::new()),
            key,
            named: named.into(),
            value_steps: value_steps.into(),
            value_heads: value_heads.into(),
        });
    }
    // Case 6rec: `form` is a MATCH ARM whose pattern is a `(record (field binder) …)` RECORD pattern
    // binding `name` at a field. A TOP-LEVEL record match destructures a record scrutinee BY FIELD (the
    // match twin of the record BINDING pattern, Increment B): a bare-binder field value `a` in `(record (x
    // a))` resolves to a PROJECTION of the scrutinee at field `x` — `Resolved::Member { operand: scrutinee,
    // key: x }`, the same `(. scrutinee x)` form that folds to a `Core::Proj` at the field's sorted slot
    // (records have no discriminant, so a top-level record arm needs no probe — the binder read is the
    // ordinary member access, zero new IR). A binder NESTED in a positional (tuple/list) field value now
    // WIRES too (§235): it resolves to a `RecordField` with an EMPTY path (the record IS the whole arm
    // pattern) reading field `key`, then descending `sub_path` into the field value — the record field
    // read lowers to `Elem(slot)`, and the positional descent appends more `Elem` steps. A record/variant
    // BELOW the field (`Unwireable`) still declines cleanly, naming the feature.
    if let Some((scrutinee, bind)) = match_arm_record_binds(db, form, from, name) {
        return Some(match bind {
            RecordArmBind::Bare(key) => Resolved::Member {
                operand: scrutinee,
                key,
            },
            RecordArmBind::Deeper(key, sub_path) => Resolved::RecordField {
                scrutinee,
                path: Vec::new().into(),
                key,
                sub_path: sub_path.into(),
                heads: Vec::new().into(),
            },
            RecordArmBind::Unwireable => Resolved::Poison(Reject::unsupported(
                "a nested compound sub-pattern inside a record match pattern is not supported \
                 (a record match binds its fields to bare names; destructure a nested field with a \
                 further `match` or `let`)",
            )),
        });
    }
    // Case 6rec-rest: `form` is a MATCH ARM whose top-level `(record (= f p) … (.. rest))` pattern binds
    // `name` as the REST binder — the RESIDUAL RECORD of the fields NOT named. Resolves to a `RecordRest`
    // (the record twin of a `MapField` REST binder): a NEW record of the scrutinee's fields minus `named`.
    // A record's field set is static, so the residual is a fixed field-subset gather (typed + folded from
    // the solved scrutinee type). Scoped to this arm.
    if let Some((scrutinee, named)) = match_arm_record_rest_binds(db, form, from, name) {
        return Some(Resolved::RecordRest {
            scrutinee,
            named: named.into(),
        });
    }
    // Case 6set-rest: `form` is a MATCH ARM whose top-level `(set e… (.. rest))` pattern binds `name` as the
    // REST binder — the RESIDUAL SET of the scrutinee MINUS the named elements. Resolves to a `SetRest` (the
    // set twin of a `MapField` REST binder / `RecordRest`): `rest : (Set E)`, the SAME set type (removing
    // elements does not narrow, UNLIKE a record). The residual VALUE is built by `desugar_runtime_set_match`
    // (a `Set.remove` chain); this Case makes the arm's `rest` reference BIND (typed `(Set E)` by infer)
    // instead of resolving CDZ0101-unbound. Scoped to this arm; `named` are the named element exprs.
    if let Some((scrutinee, named)) = match_arm_set_rest_binds(db, form, from, name) {
        return Some(Resolved::SetRest {
            scrutinee,
            named: named.into(),
        });
    }
    // Case 6rec-nested: `form` is a match arm whose pattern is a TUPLE / LIST / VARIANT compound with a
    // `(record …)` sub-pattern NESTED inside it binding `name` at a BARE-binder field — `(tuple (record (x
    // a)) c)`, `(list (record (x a)))`, `(W.Wrap (record (x a)))`. The nested walk
    // (`find_record_binder_in_pattern`) returns the access PATH to the record plus the field KEY, so the
    // `RecordField` reads field `key` off the record at that sub-path of the scrutinee (the record analogue
    // of a nested `SumPayload` / the nested-MAP Case Mn). The field itself is name-keyed (no `PathStep` step
    // for it — the slot resolves at fold from `key`), so `path` reaches the RECORD, not the field. A
    // TOP-LEVEL record arm is Case 6rec's job (`match_arm_record_binds` → `Member`), so a non-empty path is
    // required here. Complements Case 6rec; wireable BEFORE the deeper-nesting decline below.
    if let Some((scrutinee, path, key, sub_path, heads)) =
        match_arm_nested_record_binds_path(db, form, from, name)
    {
        return Some(Resolved::RecordField {
            scrutinee,
            path: path.into(),
            key,
            sub_path: sub_path.into(),
            heads: heads.into(),
        });
    }
    // Case 6rec-nested-decline: `form`'s pattern nests a `(record …)` sub-pattern binding `name` whose field
    // value is itself a COMPOUND (a further nested tuple/variant/record) — Case 6rec-nested above wired only
    // a BARE-binder field, so the deeper case falls through to here. A record field projects by NAME (no
    // name-keyed `PathStep`), so a binder BELOW a nested record field cannot yet be WIRED (the walkers skip
    // the `record` head), and the reference would otherwise fall through to a misleading CDZ0101. Resolve it
    // to the SAME clean feature-decline the top-level (Case 6rec) case gives — the deeper-nesting twin. A
    // linear pattern binds `name` once, so if it is in a nested record no earlier (wireable) case matched
    // it, making this placement safe (a genuinely-wireable binder resolved above).
    if match_arm_nested_record_binds(db, form, from, name) {
        // CODED decline (`CDZ0900`, `Reject::unsupported`) — a not-yet-built construct, NOT a REJECT. A
        // record field's value binder MAY be ANY nested pattern to any depth (core-semantics §235, the
        // record twin of the map/list nested-binder clauses), so a binder BELOW a nested record field is
        // SPEC-VALID — the compiler just does not yet wire it (no name-keyed `PathStep`). This is a graceful
        // deferral of a valid target, so it is `CDZ0900` (UnsupportedConstruct) — matching its siblings: the
        // directly-nested record-match decline (`match_arm_record_binds` → `Reject::unsupported`) and the
        // binding-position twin (`last_binder_named` + `check_binding_pattern`, CDZ0900). It was formerly
        // `Reject::coded(Code::Malformed)` = CDZ0201, which wrongly told the user a spec-valid program is
        // malformed. Still CODED (`is_decline` holds for CDZ0900), so `collect_reached_poisons` /
        // `match_pattern_fault` surfaces it in `cdz check` on EVERY body (the check≡compile property the
        // coded-head discipline requires — the concern that guided the old choice was an UNCODED decline,
        // which CDZ0900 is not).
        return Some(Resolved::Poison(Reject::unsupported(
            "a record sub-pattern nested inside a tuple/list/constructor match pattern is not \
             supported (a record match binds its fields to bare names at the top level; destructure a \
             nested record with a further `match` or `let`)",
        )));
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
    // Case R2: `form` is a `(module …)` DECLARATION form, ascended from a member's body. A PRIVATE member
    // (its name withheld from the module's `(export …)` clause) has NO synth field lambda built for it
    // (`modules::module_record` skips it), so — unlike an exported member, whose body is reparented under
    // its field lambda beneath the synth record — its body's parent chain leads up through THIS `(module …)`
    // form, never reaching the record where Case R fires. Resolving siblings here too makes a private
    // member's body see its siblings exactly as an exported member's does: the module's members are mutually
    // visible regardless of export status (the export clause governs OUTWARD reachability, not sibling
    // scope). This is the missed face of the privacy landing (0c008299) — a private member participating in
    // a mutual-recursion cycle rejected CDZ0101 at its co-member call site.
    //= spec/capabilities/modules-and-namespaces.md#visibility-is-explicit
    //# The explicit visibility rule MUST govern only a definition's reachability from outside its module, not its reachability from within.
    //= spec/capabilities/modules-and-namespaces.md#visibility-is-explicit
    //# A definition MUST remain visible to the other definitions in its own module regardless of whether it is made visible outside, so that a module's members are mutually visible and a private helper stays reachable by its siblings.
    if let Some(binder) = module_form_sibling_binds(db, form, name) {
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
    // `SumPayload` / Case B's `BinField`). Scoped to this arm. The map pattern is the DIRECT match
    // scrutinee here, so the access path is EMPTY.
    if let Some((scrutinee, key, named, value_steps, value_heads)) =
        match_arm_map_binds(db, form, from, name)
    {
        return Some(Resolved::MapField {
            scrutinee,
            path: std::rc::Rc::from(Vec::new()),
            key,
            named: named.into(),
            value_steps: value_steps.into(),
            value_heads: value_heads.into(),
        });
    }
    // Case Mn: `form`'s pattern is a COMPOUND (tuple/record/variant/list) with a MAP pattern NESTED inside
    // it that binds `name` — `((tuple (map (k v)) j) …)` binds `v` at `Elem(0)`'s map. The nested walk
    // (`find_map_binder_in_pattern`) returns the access PATH to the map plus the value key / rest keys, so
    // the `MapField` reads the map at that sub-path of the scrutinee (the map analogue of a nested
    // `SumPayload`). Complements Case M (the direct map scrutinee); tuple/list-nested tuple/list binders
    // already compose via `SumPayload`, this adds the missing nested-MAP arm.
    if let Some((scrutinee, path, key, named, value_steps, value_heads)) =
        match_arm_nested_map_binds(db, form, from, name)
    {
        return Some(Resolved::MapField {
            scrutinee,
            path: std::rc::Rc::from(path),
            key,
            named: named.into(),
            value_steps: value_steps.into(),
            value_heads: value_heads.into(),
        });
    }
    // Case Mmr: `form` is a MATCH ARM whose pattern is a `(map …)` with a MALFORMED `..` rest binding
    // `name` at a value/rest position. `map_pattern_of` gave `None` (so Cases M/Mn above did not fire), and
    // WITHOUT this the body reference would fall through to a MISLEADING CDZ0101 "unbound name" that masks
    // the real rest-shape fault (v-diagnostics note). Resolve it to the SAME coded rest-shape decline the
    // map matcher emits, SUPPRESSING the unbound cascade — the map twin of the record-pattern Case 6rec.
    // ANCHOR at the offending `(map …)` PATTERN node (not this reference), so it lands on the SAME node the
    // map matcher's reject does and the same-node dedup collapses the two into ONE primary diagnostic (the
    // list rest-shape check keeps its body binder inert, avoiding a second report; a map has no valid field
    // to bind to when malformed, so we co-anchor instead).
    if let Some(map_pat) = match_arm_malformed_map_binds(db, form, from, name) {
        return Some(Resolved::Poison(
            Reject::coded(
                crate::diag::Code::Malformed,
                "a map rest pattern is `(map (k v) … .. rest)` — exactly one binder after `..`",
            )
            .at(map_pat),
        ));
    }
    // Case Lmr: `form` is a MATCH ARM whose pattern is a `(list …)` with a MALFORMED `..` rest binding
    // `name` at an EXTRA position after `..` (`(list a .. b c)` — `c`). `lower_match_list` already faults
    // the shape (CDZ0201 at the pattern), but `find_rest_binder_in_list_pattern` recognizes only the single
    // `dd + 1` binder, so a body reference to a SURPLUS binder (`c`) fell through to a MISLEADING CDZ0101
    // "unbound name" that masks the rest-shape fault. Resolve it to the SAME coded rest-shape decline the
    // list matcher emits, SUPPRESSING the unbound cascade — the list twin of Case Mmr. Co-anchor at the
    // offending `(list …)` PATTERN node so it lands on the SAME node the matcher's reject does and the
    // same-node dedup collapses the two into ONE primary diagnostic.
    if let Some(list_pat) = match_arm_malformed_list_binds(db, form, from, name) {
        return Some(Resolved::Poison(
            Reject::coded(
                crate::diag::Code::Malformed,
                "a list rest pattern is `(list p… .. rest)` — exactly one binder after `..`",
            )
            .at(list_pat),
        ));
    }
    None
}

/// The `(list …)` PATTERN node when `form` is a MATCH ARM `(pattern body)` (ascended from its BODY, or a
/// guarded arm's guard cond) whose pattern is a `(list …)` form with a MALFORMED `..` rest
/// ([`list_form_is_malformed_rest`]) that binds `name` at a SURPLUS position after `..`; `None` otherwise.
/// The list twin of [`match_arm_malformed_map_binds`]: it lets `binder_in` resolve such a body/guard
/// reference to the clear rest-shape decline (co-anchored at the returned node) instead of leaking an
/// unbound name. Searches the arm pattern DIRECTLY and through nested tuple/variant/list payloads (a nested
/// malformed-rest list `(Wrap (list a .. b c))` reads the same way), mirroring the map companion.
fn match_arm_malformed_list_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<StructId> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    // Peel a `(guard <pattern> <cond>)` wrapper — a guard-cond reference binds the same names.
    let (arm_pat, guard_cond) = match db.ast.as_form(pb[0], "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pb[0], None),
    };
    if from != pb[1] && Some(from) != guard_cond {
        return None;
    }
    // Must be a genuine match arm (parent is `(match scrutinee arm…)`, `form` an arm, not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    match mtail.first() {
        Some(&scrutinee) if scrutinee != form => {}
        _ => return None,
    }
    // Find a MALFORMED-rest `(list …)` binding `name` at a surplus position — DIRECTLY or NESTED.
    find_malformed_list_binding_name(db, arm_pat, name)
}

/// The MALFORMED-rest `(list …)` sub-pattern of `pattern` that binds `name` at a SURPLUS post-`..` position,
/// searched DIRECTLY and through nested tuple/variant/list payloads; `None` if none. Companion of
/// [`match_arm_malformed_list_binds`] — lets a body reference to a nested malformed-list surplus binder
/// resolve to the rest-shape decline (not a leaked unbound name), the list twin of
/// [`find_malformed_map_binding_name`].
fn find_malformed_list_binding_name(db: &Db, pattern: StructId, name: &str) -> Option<StructId> {
    // A `(list …)` here: if its rest is malformed AND it binds `name` at a surplus position, this is it.
    if list_form_is_malformed_rest(db, pattern) && list_form_binds_post_rest_name(db, pattern, name)
    {
        return Some(pattern);
    }
    // Descend a compound pattern's element/payload positions (a well-formed list sub-pattern is handled by
    // the ordinary Case 6l/6r; only a malformed one reaches here). Bounded by the arena's tree depth.
    let Struct::List(children) = db.ast.get(pattern) else {
        return None;
    };
    for &child in children {
        if let Some(found) = find_malformed_list_binding_name(db, child, name) {
            return Some(found);
        }
    }
    None
}

/// The `(map …)` PATTERN node when `form` is a MATCH ARM `(pattern body)` (ascended from its BODY, or a
/// guarded arm's guard cond) whose pattern is a `(map …)` form with a MALFORMED `..` rest
/// ([`map_form_is_malformed_rest`]) that binds `name` at a value/rest position; `None` otherwise. Companion
/// of [`match_arm_map_binds`] for the malformed case: it lets `binder_in` resolve such a body/guard
/// reference to the clear rest-shape decline (co-anchored at the returned node) instead of leaking an
/// unbound name.
fn match_arm_malformed_map_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<StructId> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    // Peel a `(guard <pattern> <cond>)` wrapper — a guard-cond reference binds the same names.
    let (arm_pat, guard_cond) = match db.ast.as_form(pb[0], "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pb[0], None),
    };
    if from != pb[1] && Some(from) != guard_cond {
        return None;
    }
    // Must be a genuine match arm (parent is `(match scrutinee arm…)`, `form` an arm, not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    match mtail.first() {
        Some(&scrutinee) if scrutinee != form => {}
        _ => return None,
    }
    // Find a MALFORMED-rest `(map …)` binding `name` — DIRECTLY (the arm pattern IS the map) or NESTED
    // inside a tuple/variant/list payload (`(Wrap (map (k v) .. r (j w)))`). The nested descent mirrors
    // `find_map_binder_in_pattern`, but here we want the MALFORMED map (which that valid-binding walker
    // bails on) so its body reference resolves to the rest-shape decline instead of leaking unbound.
    find_malformed_map_binding_name(db, arm_pat, name)
}

/// The MALFORMED-rest `(map …)` sub-pattern of `pattern` that binds `name` (at a value/rest position),
/// searched DIRECTLY and through nested tuple/variant/list payloads; `None` if none. Companion of
/// [`match_arm_malformed_map_binds`] — lets a body reference to a nested malformed-map binder resolve to
/// the rest-shape decline (not a leaked unbound name), the nested twin of the direct case.
fn find_malformed_map_binding_name(db: &Db, pattern: StructId, name: &str) -> Option<StructId> {
    // A `(map …)` here: if its rest is malformed AND it binds `name`, this is the node.
    if map_form_is_malformed_rest(db, pattern) && map_form_binds_name(db, pattern, name) {
        return Some(pattern);
    }
    // Descend a compound pattern's element/payload positions (a well-formed map sub-pattern is handled by
    // the ordinary Case Mn; only a malformed one reaches here). Bounded by the arena's tree depth.
    let Struct::List(children) = db.ast.get(pattern) else {
        return None;
    };
    for &child in children {
        // Recurse into EVERY child, INCLUDING the compound HEAD (`tuple`/`list`/`map`/`record` marker, a
        // `(. Sum V)` variant head). No skip is needed: a head is a name/`.`-form with no map-binding name
        // to find, so descending into it is safe and the recursion bottoms out. (Fixed a stale comment that
        // claimed the head was skipped — it never was; PR #443.)
        if let Some(found) = find_malformed_map_binding_name(db, child, name) {
            return Some(found);
        }
    }
    None
}

/// If `form` is a match ARM `(pattern body)` (ascended from its BODY) whose pattern is a `(map (k v) …
/// .. rest)` map pattern binding `name`, return `(scrutinee, key, named)`: the enclosing match's
/// scrutinee, `Some(key-occ)` when `name` is a VALUE binder at that key (else `None` for the REST
/// binder), and `named` the keys the pattern names (removed to form the rest map). `None` otherwise.
/// The map companion of [`match_arm_bin_binds`].
#[allow(clippy::type_complexity)]
fn match_arm_map_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(
    StructId,
    Option<StructId>,
    Vec<StructId>,
    Vec<crate::core::PathStep>,
    Vec<StructId>,
)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 || from != pb[1] {
        return None; // not an arm, or the reference is not from the arm's body
    }
    // Peel a `(guard <map-pattern> <cond>)` wrapper (guarded map arm): the BODY's value/rest binders must see
    // the inner `(map …)` pattern. A bare `(map …)` reads `pb[0]` directly. (The guard COND path is Case 6mg's
    // `guard_cond_map_binds`.)
    let pattern = match db.ast.as_form(pb[0], "guard") {
        Some(g) if g.len() == 2 => g[0],
        _ => pb[0],
    };
    let (entries, rest) = map_pattern_of(db, pattern)?;
    // `form`'s parent must be a `(match scrutinee arm…)` and `form` an arm (not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    let named: Vec<StructId> = entries.iter().map(|&(k, _)| k).collect();
    // A VALUE binder: `name` is bound by the value sub-pattern of some entry → carry that entry's KEY and
    // the access sub-path INTO the value. A BARE-binder value (`(k v)`) binds `name` directly at an EMPTY
    // sub-path; a non-bare value (`(k (tuple x y))` / `(k (Some n))`) descends via the same element/payload
    // walk a list element / variant payload uses — the map analogue of Inc-1 list-element compose.
    for &(k, v) in &entries {
        if let Some((steps, heads)) = value_subpattern_binds(db, v, name) {
            return Some((scrutinee, Some(k), named, steps, heads));
        }
    }
    // The REST binder: `name` is the rest occurrence → the scrutinee minus the named keys.
    if rest.is_some_and(|r| db.ast.as_name(r).is_some_and(|nm| nm == name && nm != "_")) {
        return Some((scrutinee, None, named, Vec::new(), Vec::new()));
    }
    None
}

/// If the map-pattern VALUE sub-pattern `v` binds `name`, return `(steps, heads)`: the access sub-path from
/// the value down to the binder (empty for a bare-binder value `v`), and the variant heads at each
/// `Payload` step (for a ctor sub-pattern). `None` if `v` does not bind `name`. Reuses the same
/// element/payload descent list elements + variant payloads use (`find_binder_in_tuple`/`_list`/
/// `_pattern`), so a value MAY be a bare binder, a tuple `(tuple x y)`, a nested list, or a constructor
/// `(Some n)`, composed to any depth (`core-semantics.md §145`). A literal / wildcard value binds nothing.
fn value_subpattern_binds(
    db: &Db,
    v: StructId,
    name: &str,
) -> Option<(Vec<crate::core::PathStep>, Vec<StructId>)> {
    // A bare binder value binds `name` directly at the value itself (no sub-path).
    if let Some(nm) = db.ast.as_name(v) {
        return (nm == name && nm != "_").then(|| (Vec::new(), Vec::new()));
    }
    let mut steps = Vec::new();
    let mut heads = Vec::new();
    let found = if is_tuple_pattern(db, v) {
        find_binder_in_tuple(db, v, name, &mut steps, &mut heads)
    } else if is_list_pattern(db, v) {
        find_binder_in_list(db, v, name, &mut steps, &mut heads)
    } else {
        find_binder_in_pattern(db, v, name, &mut steps, &mut heads)
    };
    found.then_some((steps, heads))
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
    // Peel a `(guard <inner-pat> <cond>)` wrapper (§4b): a guarded bin arm `(guard (bin …) cond)` binds its
    // segment names in the BODY too, so the body reference `n` must see the inner `(bin …)` pattern. A bare
    // `(bin …)` reads `pattern` directly. (The guard COND path is Case 6bg's `guard_cond_bin_binds`.)
    let pattern = match db.ast.as_form(pattern, "guard") {
        Some(g) if g.len() == 2 => g[0],
        _ => pattern,
    };
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
    Some((scrutinee, segs.to_vec(), seg_index))
}

/// If `form` is a GUARD `(guard (bin <seg>…) <cond>)` ascended from its `<cond>`, and the bin pattern binds
/// `name` at one of its segments, return `(scrutinee, segs, seg_index)` for a `BinField` read — the BINARY
/// analogue of [`guard_cond_list_binds`]/[`guard_cond_record_binds`] (Case 6bg). `(guard (bin (u8 n)) (> n
/// 5))` binds `n` at segment 0 for the guard cond; the guard decodes it off the enclosing match's scrutinee
/// EXACTLY as the arm body does via Case B ([`match_arm_bin_binds`]). `None` otherwise. Complements Case B:
/// a reference in the guard cond ascends into the `(guard …)` form (this case) before it would reach the arm
/// (where `from` is the guard wrapper, not the cond).
fn guard_cond_bin_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<crate::resolved::Segment>, usize)> {
    // `form` must be `(guard <bin-pattern> <cond>)`, ascended from the cond.
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    let pattern = g[0];
    // Only a `(bin …)` inner pattern (a `(list …)`/`(tuple …)`/`(record …)`/variant guard is another case's
    // concern).
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
    Some((scrutinee, segs.to_vec(), seg_index))
}

/// If `form` is a GUARD `(guard (map (k v) … .. rest) <cond>)` ascended from its `<cond>`, and the map
/// pattern binds `name` (a value binder at some key, or the rest binder), return `(scrutinee, key, named,
/// value_steps, value_heads)` for a `MapField` read — the MAP analogue of [`guard_cond_bin_binds`] (Case
/// 6mg). `(guard (map (1 v)) (> v 5))` binds `v` at key `1` for the guard cond; the guard reads it off the
/// enclosing match's scrutinee EXACTLY as the arm body does via Case M ([`match_arm_map_binds`]). `None`
/// otherwise. Complements Case M: a reference in the guard cond ascends into the `(guard …)` form (this
/// case) before it would reach the arm (where `from` is the guard wrapper, not the cond).
#[allow(clippy::type_complexity)]
fn guard_cond_map_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(
    StructId,
    Option<StructId>,
    Vec<StructId>,
    Vec<crate::core::PathStep>,
    Vec<StructId>,
)> {
    // `form` must be `(guard <map-pattern> <cond>)`, ascended from the cond.
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    let pattern = g[0];
    // Only a `(map …)` inner pattern (a `(bin …)`/`(list …)`/`(tuple …)`/`(record …)`/variant guard is
    // another case's concern).
    let (entries, rest) = map_pattern_of(db, pattern)?;
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
    let named: Vec<StructId> = entries.iter().map(|&(k, _)| k).collect();
    // A VALUE binder: `name` is bound by the value sub-pattern of some entry — carry that entry's KEY and the
    // access sub-path into the value (the SAME descent [`match_arm_map_binds`]'s body path uses).
    for &(k, v) in &entries {
        if let Some((steps, heads)) = value_subpattern_binds(db, v, name) {
            return Some((scrutinee, Some(k), named, steps, heads));
        }
    }
    // The REST binder: `name` is the rest occurrence → the scrutinee minus the named keys.
    if rest.is_some_and(|r| db.ast.as_name(r).is_some_and(|nm| nm == name && nm != "_")) {
        return Some((scrutinee, None, named, Vec::new(), Vec::new()));
    }
    None
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
    // `from`'s index in `forms` (the headless tail after the `do` head). The fast path uses the O(1)
    // recorded `child_ix` (`ix - 1`, since `forms` drops the head at child index 0); the fallback recovers
    // it by IDENTITY. `child_ix_of` is `unwrap_or(0)`, so it returns 0 for the genuine do HEAD AND for any
    // node with NO recorded child-index (an unrecorded / re-parented node) — the two are indistinguishable
    // by `ix` alone. So a bare `if ix == 0 { return None }` guard would WRONGLY short-circuit a re-parented
    // `from` whose recorded index is absent (→0) BEFORE the identity fallback runs, reappearing the F2
    // false-unbound for that sub-case (PR#883 Copilot). Instead: try the fast path only when `ix >= 1` AND
    // it actually lands on `from`; otherwise ALWAYS fall to the identity scan, which is authoritative — it
    // returns the true position for a re-parented `from` (ix stale/absent) and `None` iff `from` is genuinely
    // not a direct form of this `do` (including the real head, which is not in `forms`). This preserves the
    // defensive meaning (head / non-form → None) without conflating it with the re-parent case.
    let ix = db.child_ix_of(from);
    let k = if ix >= 1 && forms.get(ix - 1) == Some(&from) {
        // Fast path: `from`'s LIVE `child_ix` still matches its position in this `do`'s forms — its parent is
        // genuinely this `do`, unchanged since load.
        ix - 1
    } else {
        // Either `from` was RE-PARENTED (its recorded `child_ix` is stale — reads a different parent — or
        // absent → 0), or `from` is the do head / not a form here. The effects fold re-parents a `do`-item
        // subtree (a `(def frame (bin …))` lifted under a fresh `let`) while a reference deep inside it (a
        // `(bin …)` build operand's `a`) still ascends here via a load-time scope-skip pointer whose `from`
        // is that re-parented item → the fast-path window would exclude the preceding do-def and the
        // reference reads UNBOUND (the F2 false CDZ0101). Recover the TRUE window by IDENTITY; a `from` that
        // is genuinely the head / not a direct form of this `do` is absent → `None` (defensive meaning kept).
        forms.iter().position(|f| *f == from)?
    };
    // The forms declaring `name`, as ascending `(position, form)` pairs. `do_forms_declaring` gives them
    // in O(1) for a LOAD-TIME do-block (the per-block declaration index, `Db::do_binder_index`) — so a
    // reference to a name the block never declares (a prelude/outer name — `+`, `unit`, a member access's
    // own head, BY FAR the common case) is an O(1) miss, not the old per-reference O(forms) scan that made
    // a wide `(do (def …)… <N uses>)` O(N²). It returns `None` only for a do-block APPENDED AFTER load (a
    // copied recursive-do-local body from inlining, past the load-time arena — not in the index); there we
    // fall back to a LIVE scan of the forms building the same candidate list (correctness over speed — a
    // copied body is small and re-resolved a bounded number of times).
    let owned;
    let cands: &[(u32, StructId)] = match db.do_forms_declaring(form, name) {
        Some(indexed) => indexed,
        None => {
            owned = forms
                .iter()
                .enumerate()
                .filter(|&(_, &f)| {
                    let is_def = db
                        .ast
                        .as_form(f, "def")
                        .and_then(|t| t.first())
                        .map(|&sig| match db.ast.get(sig) {
                            Struct::Atom(_) => db.ast.as_name(sig) == Some(name),
                            Struct::List(children) => {
                                children.first().and_then(|&c| db.ast.as_name(c)) == Some(name)
                            }
                        })
                        .unwrap_or(false);
                    let is_mod = db
                        .ast
                        .as_form(f, "module")
                        .and_then(|t| t.first())
                        .and_then(|&n| db.ast.as_name(n))
                        == Some(name);
                    is_def || is_mod
                })
                .map(|(pos, &f)| (pos as u32, f))
                .collect::<Vec<_>>();
            &owned
        }
    };
    // Reverse (sequential) scope: the LAST declaration of `name` at a position < `k` wins (a form sees
    // earlier declarations, not itself or later ones). `cands` is ascending, so `partition_point` finds
    // the highest such position — byte-identical to the old reverse scan's first-hit-walking-backward.
    let before = cands.partition_point(|&(pos, _)| (pos as usize) < k);
    if let Some(&(_, f)) = before.checked_sub(1).map(|last| &cands[last]) {
        // The same per-form logic the old reverse scan ran (a `def` binder, else a `(module …)` record).
        if let Some(binder) = do_def_binds(db, f, name) {
            return Some(binder);
        }
        // A do-local `(module NAME …)` binds `NAME` to its synthesized record (fields = its exported defs,
        // built at load by `modules::synthesize`) — a `Ref` to the record, so `(. NAME field)` is ordinary
        // member access.
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
    // or the top-level defs, not strictly sequential like a value binding. So check EVERY declaring form
    // (including `from` itself and the ones after it) for a FUNCTION def of `name` — accepting ONLY a
    // `Lambda` (a `(def (f p…) BODY)` with parameters), never a `Ref`. This keeps a VALUE def strictly
    // backward (`(do (def x 5) (def x (+ x 10)) x)` = 15 — the second `x` sees only the first, not
    // itself), while a recursive/forward FUNCTION reference resolves. First (lowest-position) match wins.
    cands
        .iter()
        .filter_map(|&(_, f)| match do_def_binds(db, f, name) {
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
    // reverse of `modules::synthesize`), then scan its members.
    let module_form = db.module_by_synth_record(form)?;
    module_members_bind(db, module_form, name)
}

/// If `form` is a `(module …)` DECLARATION form, resolve `name` against its members — the twin of
/// `module_sibling_binds` for a body that ascends through the module FORM itself, not its synth record.
/// A member whose field is WITHHELD from the export record (a PRIVATE member — its name is absent from
/// the module's `(export …)` clause) is NOT reparented under a synth field lambda, so its body's parent
/// chain leads up through the `(module …)` form and NEVER reaches the synth record where
/// `module_sibling_binds` (Case R) fires. Without this case a private member could not see ANY sibling
/// (exported or private) — the false-CDZ0101 the privacy landing (0c008299) left for a private member
/// participating in a cycle, since `modules::module_record` builds no field for it. A module's members
/// are mutually visible regardless of export status (`modules-and-namespaces.md` §Visibility Is Explicit:
/// the export clause governs OUTWARD reachability through the record, not sibling visibility), so this
/// scans ALL members exactly as the record path does. `None` if `form` is not a module form or no member
/// is named `name`.
fn module_form_sibling_binds(db: &Db, form: StructId, name: &str) -> Option<Resolved> {
    db.ast.as_form(form, "module")?;
    module_members_bind(db, form, name)
}

/// The shared member scan behind `module_sibling_binds` (from the synth record) and
/// `module_form_sibling_binds` (from the `(module …)` form): resolve `name` against the module's members,
/// mutually-visible with no stop-before. `module_form` is the `(module NAME member…)` declaration
/// occurrence; its members are the tail after NAME.
fn module_members_bind(db: &Db, module_form: StructId, name: &str) -> Option<Resolved> {
    let members = db.ast.as_form(module_form, "module")?.get(1..)?;
    // O(1) via the per-module member index (`Db::do_binder_index` also indexes `(module …)` forms) — the
    // members declaring `name`, ascending. A name no member declares is absent → an O(1) negative; before,
    // this `find_map`-scanned ALL members on EVERY sibling/member reference, so a wide module referenced
    // from N sites was O(members × refs) = O(N²) (the do-scope trap of this same fix, in the module scope).
    // A module APPENDED after load (a copied body from inlining, not in the index) falls back to a live
    // member scan — the same correctness-over-speed fallback the do-scope uses.
    let owned;
    let cands: &[(u32, StructId)] = match db.do_forms_declaring(module_form, name) {
        Some(indexed) => indexed,
        None => {
            owned = members
                .iter()
                .enumerate()
                .map(|(pos, &m)| (pos as u32, m))
                .collect::<Vec<_>>();
            &owned
        }
    };
    // FIRST-wins across the members (a duplicate name is a separate concern; the sum/def indices resolve a
    // shared name first-wins too), so take the LOWEST-position declaring member. A NESTED `(module inner …)`
    // member binds `inner` to its synthesized record — the same `Ref` the `(. outer inner)` projection folds
    // to — so a sibling def's body may reference the inner module by bare name, exactly as a sibling def.
    cands.iter().find_map(|&(_, m)| {
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
    // `form` must be a handle arm (parent shape) whose BODY is `from`. Two shapes: the 4-element tail/
    // abortive arm `(op (params) state body)` (body = parts[3]) and the 5-element general `ctl`-style arm
    // `(op (params) state k body)` (body = parts[4], with `k` the continuation binder at parts[3]).
    let Struct::List(parts) = db.ast.get(form) else {
        return None;
    };
    if !is_handle_arm(db, form) {
        return None;
    }
    // Locate the body slot + the extra continuation binder (if any) by arm arity.
    let (body_ix, cont_binder) = match parts.len() {
        4 => (3, None),
        5 => (4, Some(parts[3])),
        _ => return None,
    };
    if parts[body_ix] != from {
        return None;
    }
    // The CONTINUATION binder `k` (element 3 of a 5-part arm) — a bare name binding the reified delimited
    // continuation as a value. Shadows the state/params (the innermost binder for that name).
    if let Some(k) = cont_binder
        && db.ast.as_name(k) == Some(name)
        && name != "_"
    {
        return Some(k);
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

/// Whether the `resume` at `node` is STRAY — a LIVE source `resume` that is NOT inside a handler arm's
/// body. A `resume` hands a value back to the point that performed the arm's operation, so it is meaningful
/// ONLY inside a handler arm's BODY; anywhere else it has no arm to return into and is rejected (a coded
/// diagnostic, not a silent lowering decline). Walking UP the parent chain: (a) if an ancestor we came from
/// is element 3 (the body) of a 4-element list `is_handle_arm` recognizes, the `resume` is well-placed (not
/// stray); (b) the chain must reach the arena ROOT — a SYNTHESIZED fold copy (a `push_list`/`beta_reduce`
/// node the reduction produced, root-parent `None`) is NOT a live source node and must NOT be flagged (its
/// chain dead-ends before the root). So a `resume` is stray iff its chain reaches the root WITHOUT passing
/// through a handler-arm body. Bounded by the finite parent chain.
pub(crate) fn is_stray_resume(db: &Db, node: StructId) -> bool {
    let mut child = node;
    while let Some(parent) = db.parent_of(child) {
        // A handler-arm BODY is the last element of a 4-part (tail/abortive) or 5-part (general ctl-style)
        // arm. A `resume` inside that body is well-placed. (A 5-part arm's body is parts[4], its `k` binder
        // parts[3]; a `resume` there is still well-placed — the general arm's lowering consumes it as
        // `apply(k, v)` once the E5 increment lands.)
        if let Struct::List(parts) = db.ast.get(parent)
            && matches!(parts.len(), 4 | 5)
            && parts[parts.len() - 1] == child
            && is_handle_arm(db, parent)
        {
            return false; // well-placed inside a handler arm body
        }
        child = parent;
    }
    // The chain ended. If it reached the arena root, this is a LIVE source `resume` with no enclosing arm →
    // stray. If it dead-ended before the root, it is a synthesized fold copy — NOT flagged.
    child == db.ast.root
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
    // A `(bin …)` inner pattern is NOT a variant/ctor — its `(u8 n)` etc. are binary SEGMENTS, not nested
    // constructor payloads. `find_binder_in_pattern` would otherwise walk `(bin (u8 n))` as a ctor and bind
    // `n` at a spurious `[Payload, Payload]` sum path (reading the Bytes handle as a boxed sum → garbage),
    // shadowing Case 6bg's correct `BinField`. Exclude it here (the bin guard cond is `guard_cond_bin_binds`),
    // exactly as this case already excludes `tuple`/`list`/`record` heads (Cases 6tg/6lg/6recg).
    if db.ast.head_name(pattern) == Some("bin") {
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

/// If `form` is a guard `(guard <compound-pattern> <cond>)` ascended from its `<cond>`, and the inner
/// compound (variant/tuple/list) pattern NESTS a `(record …)` sub-pattern binding `name` at a bare-binder
/// field, the `(scrutinee, path-to-record, field-key, heads)` for the `RecordField` read — the guard-cond
/// twin of Case 6rec-nested ([`match_arm_nested_record_binds_path`]). `(guard (Some (record (x a))) (> a 0))`
/// binds `a` (path `[Payload]`, head `Some`, key `x`) for the guard cond. A guard-cond reference ascends into
/// the `(guard …)` form (THIS case) before it would reach the arm — where `from` is the guard WRAPPER, not
/// the cond, so `match_arm_nested_record_binds_path`'s `from == guard_cond` check cannot fire for a guard-cond
/// reference. A TOP-LEVEL record guard is Case 6recg's ([`guard_cond_record_binds`]), so a non-empty `path`
/// is required here. `None` otherwise. (`find_record_binder_in_pattern` — the descent Case 6rec-nested uses —
/// walks variant/tuple/list compounds into a nested record; a bare `find_binder_in_pattern`, which
/// `guard_cond_variant_binds` uses, skips the `record` head, which is why the plain variant guard case misses
/// this and it needs its own twin.)
fn guard_cond_nested_record_binds_path(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(
    StructId,
    Vec<crate::core::PathStep>,
    Symbol,
    Vec<crate::core::PathStep>,
    Vec<StructId>,
)> {
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    let pattern = g[0];
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
    // Descend the compound pattern for a NESTED `(record …)` binding `name` — the same walk Case 6rec-nested
    // uses; `path` reaches the RECORD (the field is name-keyed via the returned key). A non-empty path is
    // required (a TOP-LEVEL record guard is Case 6recg's).
    let mut path = Vec::new();
    let mut heads = Vec::new();
    let (key, sub_path) = find_record_binder_in_pattern(db, pattern, name, &mut path, &mut heads)?;
    if path.is_empty() {
        return None; // the record is the whole guard pattern → Case 6recg's job
    }
    Some((scrutinee, path, key, sub_path, heads))
}

/// If `form` is a guard `(guard (list p… [.. rest]) <cond>)` ascended from its `<cond>`, and the LIST
/// pattern binds `name` (a leading element or the rest binder), the `(scrutinee, path, heads)` for the
/// `SumPayload` read — the LIST analogue of [`guard_cond_variant_binds`]. `(guard (list x .. rest) (> x
/// 0))` binds `x` at `[Elem(0)]` and `rest` at `[RestFrom(1)]` for the guard cond. `None` otherwise.
/// Complements Cases 6l/6r: a reference in the guard cond ascends into the `(guard …)` form (this case)
/// before it would reach the arm (where `from` is the guard wrapper, not the cond).
fn guard_cond_list_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<crate::core::PathStep>, Vec<StructId>)> {
    // `form` must be `(guard <list-pattern> <cond>)`, ascended from the cond.
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    let pattern = g[0];
    // Only a `(list …)` inner pattern (a `(map …)`/variant/tuple guard is another case's concern).
    // `compound_form_of` recognizes the native `#list(…)` ctor-leaf head too (not only the name/string alias);
    // `?` bails when it is not a list pattern (clippy::question_mark — the value itself is unused here).
    db.ast
        .compound_form_of(pattern, crate::ast::CompoundCtor::List)?;
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
    // A LEADING element sub-pattern (path `[Elem(i), …]`, possibly nested) or the REST binder
    // (`[RestFrom(lead)]`) — the same descent Cases 6l/6r use over the arm body.
    if let Some((path, heads)) = find_leading_binder_in_list_pattern(db, pattern, name) {
        return Some((scrutinee, path, heads));
    }
    let lead = find_rest_binder_in_list_pattern(db, pattern, name)?;
    Some((
        scrutinee,
        vec![crate::core::PathStep::RestFrom(lead)],
        Vec::new(),
    ))
}

/// If `form` is a guard `(guard (tuple p0 p1 …) <cond>)` ascended from its `<cond>`, and the TUPLE pattern
/// binds `name` at some element (possibly nested), the `(scrutinee, path, heads)` for a `SumPayload` read —
/// the TUPLE analogue of [`guard_cond_list_binds`]. `(guard (tuple a b) (> (+ a b) 0))` binds `a` at
/// `[Elem(0)]` and `b` at `[Elem(1)]` for the guard cond. `None` otherwise. Reuses [`find_binder_in_tuple`]
/// (the same descent the arm BODY uses via Case 6), which recurses into nested tuple/list/ctor sub-patterns.
/// Complements Case M/6: a reference in the guard cond ascends into the `(guard …)` form (this case) before
/// it would reach the arm (where `from` is the guard wrapper, not the cond).
fn guard_cond_tuple_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<crate::core::PathStep>, Vec<StructId>)> {
    // `form` must be `(guard <tuple-pattern> <cond>)`, ascended from the cond.
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    let pattern = g[0];
    // Only a `(tuple …)` inner pattern (a `(list …)`/`(map …)`/variant guard is another case's concern).
    if !is_tuple_pattern(db, pattern) {
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
    // Descend the tuple pattern to find where `name` is bound (its element path + per-step heads).
    let mut path = Vec::new();
    let mut heads = Vec::new();
    find_binder_in_tuple(db, pattern, name, &mut path, &mut heads)
        .then_some((scrutinee, path, heads))
}

/// The (key-occurrence, sub-pattern) of a record-PATTERN field, handling both the canonical
/// `(= key sub-pattern)` triple (path B — key = child 1, sub-pattern = child 2) and a legacy
/// `(key sub-pattern)` pair (key = child 0). `None` if neither shape. Shared by every record-pattern
/// field reader (guard-cond binds, Case 6rec, the let-binder resolver's nested path) so the triple vs
/// pair distinction lives in ONE place.
fn record_pattern_field_kv(db: &Db, field: StructId) -> Option<(StructId, StructId)> {
    // A trailing `(.. rest)` rest marker is NOT a field — its head is `..`, and the legacy 2-element case
    // below would otherwise mis-read it as field `..` binding `rest` (a spurious `Member { key: .. }` →
    // CDZ0212 "record has no field `..`"). The rest binder is resolved separately (Case 6rec-rest →
    // `Resolved::RecordRest`); skip it here so every field iteration ignores the marker.
    if db.ast.as_name(field).is_none()
        && let Struct::List(kv) = db.ast.get(field)
        && kv.first().is_some_and(|&h| db.ast.as_name(h) == Some(".."))
    {
        return None;
    }
    match db.ast.get(field) {
        Struct::List(kv) if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") => {
            Some((kv[1], kv[2]))
        }
        Struct::List(kv) if kv.len() == 2 => Some((kv[0], kv[1])),
        _ => None,
    }
}

/// If `form` is a GUARD `(guard (record (= x a) …) <cond>)`, ascended from the cond, whose record pattern
/// binds `name` at a BARE-binder field, return `(scrutinee, field-key)` — the enclosing match's scrutinee
/// and the field label, so a guard-cond reference resolves to a `Member` projection (the record analogue
/// of [`guard_cond_tuple_binds`], Case 6recg). A field binder is a `(= field <name>)` VALUE position; the
/// KEY is a label, never a binder. A NESTED compound field value is not returned here (Case 6rec's body
/// path handles the decline); only the wireable bare-binder field is resolved in the guard cond.
fn guard_cond_record_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Symbol)> {
    // `form` must be `(guard <record-pattern> <cond>)`, ascended from the cond.
    let g = db.ast.as_form(form, "guard")?;
    if g.len() != 2 || g[1] != from {
        return None;
    }
    let pattern = g[0];
    let fields = db.ast.compound_form_of(pattern, CompoundCtor::Record)?;
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
    // A bare-binder field value `(= x a)` binding `name` → the field key, for a `Member` projection.
    for &pair in fields {
        if let Some((key_id, binder_id)) = record_pattern_field_kv(db, pair)
            && db.ast.as_name(binder_id) == Some(name)
            && name != "_"
            && let Some(key) = read_key(db, key_id)
        {
            return Some((scrutinee, key));
        }
    }
    None
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
    // A bare name that IS a NULLARY VARIANT CONSTRUCTOR is the constructor, not a binder — a `(match a
    // (TInt …) (TBool …))` arm head `TInt` matches the variant refutably and binds NOTHING (lowering reads
    // it via `variant_disc_by_name`, and a bare variant reference resolves to its ctor). Without this, an
    // outer nullary-variant arm was treated as binding its own name, so a NESTED bare nullary-variant match
    // (`(TInt (match b (TInt …) (TBool …)))`) had its inner `TInt` resolve — scope-FIRST — to the "binder"
    // the outer arm appeared to introduce, instead of to the variant ctor. That mis-resolution then drew
    // spurious CDZ0306 "unused binding `TInt`" + CDZ0213 "unreachable arm" warnings (the guard in
    // `collect_unused` keys off `variant_disc_of`, which the mis-resolved occurrence failed). A bare
    // variant name never binds, so returning `None` here is correct at every match nesting depth. (An
    // APPLIED variant `(Mk x)` is a `Struct::List`, so `as_name(binder_pat)` is `None` above and never
    // reaches here — only a lone bare name does. `variant_ctor_by_name` is the same O(1) load-time index
    // `resolve_name`'s bare-variant step consults, so the classification agrees with resolution.)
    if db.variant_ctor_by_name(pat_name).is_some() {
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

/// Whether `form` is a MATCH ARM `(pattern body)` (ascended from `body`, or a guarded arm's guard cond)
/// whose pattern is a `(record (field binder) …)` RECORD pattern that binds `name` at one of its field
/// positions — returning HOW to resolve the reference. A TOP-LEVEL record match `(match r ((record (x a)
/// (y b)) …))` destructures a record scrutinee BY FIELD, the match twin of the record BINDING pattern
/// (Increment B): a field's bare-binder value `a` resolves to a PROJECTION of the scrutinee at that field
/// — `(. scrutinee x)` = `Resolved::Member { operand: scrutinee, key: x }`, folding to a `Core::Proj` at
/// the field's sorted slot exactly as the let-binder record arm does (`last_binder_named`). A record has
/// NO discriminant (like a tuple), so a top-level record arm imposes no probe and the binder read is the
/// ordinary member access.
///
/// Returns `Some((scrutinee, Some(key)))` when `name` is a BARE-binder field value (resolve to a `Member`
/// projection at `key`); `Some((scrutinee, None))` when `name` is bound NESTED inside a field's COMPOUND
/// value (`(record (p (tuple a b)))`), which cannot yet be WIRED — a record field projects by name→slot
/// and a nested descent needs a composed path `PathStep` has no name-keyed step for, so the caller declines
/// cleanly naming the feature rather than emitting a misleading CDZ0101; and `None` when `name` is not
/// bound by this record pattern (a genuinely-unbound reference falls through to its real CDZ0101). GATED to
/// a genuine match arm over a record pattern, so a `(record …)` VALUE expression (not a pattern) is
/// untouched.
/// How a top-level `(record …)` match arm binds `name`: a BARE field (→ a `Member` projection), a DEEPER
/// positional field value (→ a `RecordField` reading field `key` then descending `sub_path`, §235), or an
/// UNWIREABLE deeper field (a record/variant BELOW the field — not yet composable, → the coded decline).
enum RecordArmBind {
    Bare(Symbol),
    Deeper(Symbol, Vec<crate::core::PathStep>),
    Unwireable,
}

fn match_arm_record_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, RecordArmBind)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    let (pattern, body) = (pb[0], pb[1]);
    // Peel a `(guard <record-pattern> <cond>)` wrapper — a reference in the guard cond binds the same
    // field binders as the body (the record analogue of the tuple/list guard peels).
    let (record_pat, guard_cond) = match db.ast.as_form(pattern, "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pattern, None),
    };
    if from != body && Some(from) != guard_cond {
        return None;
    }
    // The pattern must be a `(record (key value) …)` form.
    let fields = db.ast.compound_form_of(record_pat, CompoundCtor::Record)?;
    // `form` must be an arm of an enclosing `(match scrutinee arm…)`, not the scrutinee itself.
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = match mtail.first() {
        Some(&s) if s != form => s,
        _ => return None,
    };
    // A BARE-binder field value `(= x a)` binding `name` → a projection of the scrutinee at field `x`.
    for &pair in fields {
        if let Some((key_id, binder_id)) = record_pattern_field_kv(db, pair)
            && db.ast.as_name(binder_id) == Some(name)
            && name != "_"
            && let Some(key) = read_key(db, key_id)
        {
            return Some((scrutinee, RecordArmBind::Bare(key)));
        }
    }
    // Otherwise: is `name` bound NESTED inside a field's compound value (§235)? A TUPLE/LIST field value is
    // positional — the binder descends by `Elem` (+ `Payload` for a variant), which composes into a
    // `RecordField` `sub_path` (Deeper). A nested RECORD field value would need a deferred name-keyed slot,
    // and a variant below the field needs a `sub_heads` entry `RecordField` does not carry — both are NOT
    // yet wired (Unwireable → the caller's coded decline). A name bound by NO field returns `None` → its
    // real CDZ0101.
    for &pair in fields {
        if let Some((key_id, value_pat)) = record_pattern_field_kv(db, pair) {
            let (mut sub_path, mut sub_heads) = (Vec::new(), Vec::new());
            let bound = if is_tuple_pattern(db, value_pat) {
                find_binder_in_tuple(db, value_pat, name, &mut sub_path, &mut sub_heads)
            } else if is_list_pattern(db, value_pat) {
                find_binder_in_list(db, value_pat, name, &mut sub_path, &mut sub_heads)
            } else if db
                .ast
                .compound_form_of(value_pat, crate::ast::CompoundCtor::Record)
                .is_some()
            {
                record_pattern_binds_name(db, value_pat, name)
            } else {
                find_binder_in_pattern(db, value_pat, name, &mut sub_path, &mut sub_heads)
            };
            if bound {
                // Wireable iff the descent is all-`Elem` (no `Payload` → empty `sub_heads`) and the field
                // key resolves — the tuple/list-below-field case. Else Unwireable (record/variant below).
                let bind = match read_key(db, key_id) {
                    Some(key) if sub_heads.is_empty() => RecordArmBind::Deeper(key, sub_path),
                    _ => RecordArmBind::Unwireable,
                };
                return Some((scrutinee, bind));
            }
        }
    }
    None
}

/// Whether `form` is a match ARM whose TOP-LEVEL `(record (= f p) … (.. rest))` pattern binds `name` as the
/// REST binder — the residual record of the fields NOT named by the pattern. Returns `(scrutinee, the named
/// field-name key occurrences)`; the residual `rest` is the scrutinee's record minus those. The record twin
/// of the map-rest arm (Case M with `key = None`). `None` when `name` is not this arm's record-rest binder
/// (no trailing `.. rest`, or the rest names a different binder).
fn match_arm_record_rest_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<StructId>)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    let (pattern, body) = (pb[0], pb[1]);
    let (record_pat, guard_cond) = match db.ast.as_form(pattern, "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pattern, None),
    };
    if from != body && Some(from) != guard_cond {
        return None;
    }
    let fields: Vec<StructId> = db
        .ast
        .compound_form_of(record_pat, CompoundCtor::Record)?
        .to_vec();
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = match mtail.first() {
        Some(&s) if s != form => s,
        _ => return None,
    };
    // Split the trailing `.. rest`; its operand must be the bare binder `name`.
    let (leads, rest): (&[StructId], StructId) = match db.ast.rest_marker(&fields) {
        Some((k, operand, trailing_start)) if trailing_start == fields.len() => {
            (&fields[..k], operand)
        }
        _ => return None,
    };
    if db.ast.as_name(rest) != Some(name) || name == "_" {
        return None;
    }
    // The NAMED field key occurrences (removed from the scrutinee's record to form the residual `rest`).
    let mut named = Vec::new();
    for &pair in leads {
        if let Some((key_id, _)) = record_pattern_field_kv(db, pair) {
            named.push(key_id);
        }
    }
    Some((scrutinee, named))
}

/// Whether `form` is a match ARM `(pattern body)` (ascended from its BODY or guard cond) whose top-level
/// `(set e… (.. rest))` pattern binds `name` as the REST binder — returning `(scrutinee, named_elems)`.
/// The set twin of [`match_arm_record_rest_binds`], SIMPLER because a set element is an ordinary value
/// expression (NOT a key/value pair): `named` is the leading element exprs verbatim. The rest binder must be
/// the bare `name` (not `_`), following exactly one `..` at the tail. `None` otherwise.
fn match_arm_set_rest_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<StructId>)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    let (pattern, body) = (pb[0], pb[1]);
    let (set_pat, guard_cond) = match db.ast.as_form(pattern, "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pattern, None),
    };
    if from != body && Some(from) != guard_cond {
        return None;
    }
    let elems: Vec<StructId> = db
        .ast
        .compound_form_of(set_pat, CompoundCtor::Set)?
        .to_vec();
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = match mtail.first() {
        Some(&s) if s != form => s,
        _ => return None,
    };
    // Split the trailing `.. rest`; its operand must be the bare binder `name`.
    let (leads, rest): (&[StructId], StructId) = match db.ast.rest_marker(&elems) {
        Some((k, operand, trailing_start)) if trailing_start == elems.len() => {
            (&elems[..k], operand)
        }
        _ => return None,
    };
    if db.ast.as_name(rest) != Some(name) || name == "_" {
        return None;
    }
    // A set element is a value expression (not a key/value pair) — the named elements verbatim.
    Some((scrutinee, leads.to_vec()))
}

/// Whether `form` is a match ARM `(pattern body)` (ascended from its BODY or guard cond) whose pattern is
/// a TUPLE / LIST / VARIANT compound (NOT a top-level record — that's [`match_arm_record_binds`]) that
/// contains a `(record …)` sub-pattern NESTED inside it binding `name`. A record field projects by NAME
/// and `PathStep` has no name-keyed step to compose a record projection under a tuple/list/variant descent,
/// so such a nested-record binder cannot yet be WIRED — `find_binder_in_tuple`/`_list`/`_pattern` skip the
/// `record` head and the body reference falls through to a misleading CDZ0101 "unbound name". This detector
/// lets `binder_in` resolve it to the SAME clean feature-decline the top-level (Case 6rec) + let cases
/// give, so a nested-record match names the unimplemented feature rather than blaming the user (the
/// nested-in-compound twin of Case 6rec). `true` when `name` is bound inside such a nested record pattern.
fn match_arm_nested_record_binds(db: &Db, form: StructId, from: StructId, name: &str) -> bool {
    let Struct::List(pb) = db.ast.get(form) else {
        return false;
    };
    if pb.len() != 2 {
        return false;
    }
    let (pattern, body) = (pb[0], pb[1]);
    // Peel a `(guard <pattern> <cond>)` wrapper — a guard-cond reference binds the same nested binders.
    let (arm_pat, guard_cond) = match db.ast.as_form(pattern, "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pattern, None),
    };
    if from != body && Some(from) != guard_cond {
        return false;
    }
    // `form` must be an arm of an enclosing `(match scrutinee arm…)`, not the scrutinee itself.
    let Some(parent) = db.parent_of(form) else {
        return false;
    };
    let Some(mtail) = db.ast.as_form(parent, "match") else {
        return false;
    };
    match mtail.first() {
        Some(&scrutinee) if scrutinee != form => {}
        _ => return false,
    }
    // The arm pattern must be a TUPLE / LIST / VARIANT compound — a TOP-LEVEL record is Case 6rec's, not
    // here (its own bare-binder fields wire; only a NESTED record under a compound is the unwired case).
    let is_top_record = db
        .ast
        .compound_form_of(arm_pat, crate::ast::CompoundCtor::Record)
        .is_some();
    if is_top_record {
        return false;
    }
    // Walk the arm pattern for a `(record …)` sub-pattern that binds `name`.
    pattern_has_nested_record_binding(db, arm_pat, name)
}

/// Whether the pattern subtree `pat` contains a `(record …)` sub-pattern binding `name` — a recursive walk
/// over tuple/list/variant/record compounds. Used by [`match_arm_nested_record_binds`] to attribute an
/// otherwise-unbound reference to an unwired nested-record match sub-pattern (the CDZ0101-suppression
/// discipline). A `(record …)` node binding `name` (via [`record_pattern_binds_name`]) is the hit; any
/// other compound is walked into its children.
fn pattern_has_nested_record_binding(db: &Db, pat: StructId, name: &str) -> bool {
    // A record node that binds `name` — the hit (this is the unwireable nested case).
    if db
        .ast
        .compound_form_of(pat, crate::ast::CompoundCtor::Record)
        .is_some()
        && record_pattern_binds_name(db, pat, name)
    {
        return true;
    }
    // Otherwise recurse into a compound's children (tuple/list elements, variant payload args, record
    // field values, guard inner). A bare atom / literal / non-binding leaf is not a record hit.
    let Struct::List(children) = db.ast.get(pat) else {
        return false;
    };
    children
        .iter()
        .any(|&c| pattern_has_nested_record_binding(db, c, name))
}

/// Descend a LIST PATTERN `(list p… [.. rest])` looking for a LEADING element binder `name` — the
/// form-independent core shared by the match-arm path ([`list_pattern_element_binds`]) and the
/// `let`/param binding path ([`last_binder_named`]'s list arm). `list_pat` is the `(list …)` node itself.
/// Returns `Some((path, heads))` when a leading element sub-pattern binds `name`: the path is `Elem(i)`
/// (into leading position `i`) then the steps the element sub-pattern imposes (a `(tuple a b)` element adds
/// `Elem(j)`, a `(Mk x)` element adds `[Payload]`), and `heads` the variant heads at each `Payload` step.
/// The REST binder (after `..`) is [`find_rest_binder_in_list_pattern`], not here. `None` if `name` is not
/// bound at a leading position. Element positions compose to any depth (`core-semantics.md §145`).
fn find_leading_binder_in_list_pattern(
    db: &Db,
    list_pat: StructId,
    name: &str,
) -> Option<(Vec<crate::core::PathStep>, Vec<StructId>)> {
    // FAST PATH: a SIMPLE pattern (every leading element a bare name / `_`, no nested tuple/variant/list)
    // has its binders indexed ONCE by name (`Db::simple_list_binders`), so this lookup is an O(1) map read
    // rather than the O(leading) scan below — the O(N²) fix (a wide `(list a0 … aN .. r)` referenced N
    // times). A simple leading binder's path is a lone `Elem(i)` with NO variant head, so `heads` is empty.
    if let Some(binders) = simple_list_binders(db, list_pat) {
        return binders
            .by_name
            .get(name)
            // A rest binder is `[RestFrom(lead)]`, a leading binder `[Elem(i)]` — filter to the LEADING
            // ones here (the rest binder is `find_rest_binder_in_list_pattern`'s concern), so this returns
            // exactly what the linear descent did.
            .filter(|steps| matches!(steps.first(), Some(crate::core::PathStep::Elem(_))))
            .map(|steps| (steps.clone(), Vec::new()));
    }
    // SLOW PATH (a pattern with a nested element — never one of the two measured explosions, and a wide
    // nested-element list arm declines at lowering anyway): the exact element-by-element descent.
    let elems = db.ast.compound_form_of(list_pat, CompoundCtor::List)?;
    // The LEADING positions are those before a `..` marker (all of them for a fixed-arity pattern).
    let lead = db
        .ast
        .rest_marker(elems)
        .map(|(i, _, _)| i)
        .unwrap_or(elems.len());
    for (i, &elem) in elems[..lead].iter().enumerate() {
        #[cfg(test)]
        crate::db::LIST_PATTERN_BINDER_ELEMS_SCANNED.with(|c| c.set(c.get() + 1));
        let mut path = vec![crate::core::PathStep::Elem(i)];
        let mut heads = Vec::new();
        let found = if let Some(elem_name) = db.ast.as_name(elem) {
            elem_name == name && elem_name != "_"
        } else if is_tuple_pattern(db, elem) {
            find_binder_in_tuple(db, elem, name, &mut path, &mut heads)
        } else if is_list_pattern(db, elem) {
            // A NESTED LIST element `(list (list a .. r1) .. r2)` — the element at `Elem(i)` is itself a
            // list pattern, so descend it with `find_binder_in_list` (which handles nested leading elements
            // AND a `.. rest` sublist via `RestFrom`). Its own `Elem`/`RestFrom` steps stack onto the outer
            // `Elem(i)`, so `a` reads `Elem(i), Elem(0)` and `r1` reads `Elem(i), RestFrom(…)`. `find_binder_
            // in_pattern` excludes the `list` head (it is a compound-value ctor, not a variant), so without
            // this branch a nested-list element's binder fell through unresolved (CDZ0101 in the body).
            find_binder_in_list(db, elem, name, &mut path, &mut heads)
        } else {
            find_binder_in_pattern(db, elem, name, &mut path, &mut heads)
        };
        if found {
            return Some((path, heads));
        }
    }
    None
}

/// The SIMPLE list-pattern binder index for `list_pat` (`Db::simple_list_binders`), built once and read
/// O(1) thereafter. `Some(binders)` iff `list_pat` is a `(list …)` pattern whose EVERY leading element
/// (before an optional `.. rest`) is a bare-name binder or `_` — the shape both O(N²) explosions take (a
/// wide user destructure `(list a0 … aN .. r)` and the synth `(list __le0 … __leN .. r)` of the refutable-
/// literal-element desugar). `None` for a pattern with ANY nested (tuple/variant/list) leading element:
/// its binders live at deeper paths that the linear descent (`find_binder_in_*`) enumerates exactly, so
/// the caller falls back to that (whose verdict is byte-identical). Both outcomes cache, so a repeated
/// resolution against the same pattern never re-scans it. Interior mutation via the `RefCell` is sound —
/// the compiler is single-threaded (`!Send`), the arena is append-only (a pattern's shape never changes),
/// and the map only grows (an entry, once built, is stable). Returns a cheap `Rc` clone of the entry.
fn simple_list_binders(
    db: &Db,
    list_pat: StructId,
) -> Option<std::rc::Rc<crate::db::SimpleListBinders>> {
    if let Some(hit) = db.simple_list_binders.borrow().get(&list_pat) {
        return hit.clone();
    }
    let built = build_simple_list_binders(db, list_pat);
    db.simple_list_binders
        .borrow_mut()
        .insert(list_pat, built.clone());
    built
}

/// Enumerate a SIMPLE `(list …)` pattern's binders in ONE pass — every leading bare-name binder at
/// `[Elem(i)]` plus a trailing `.. rest` binder at `[RestFrom(lead)]`. Returns `None` (not simple) the
/// moment a leading element is anything but a bare name / `_` / the `..` marker, so the caller keeps the
/// linear descent for nested-element patterns. `_` is not a binder (skipped); a repeated name is last-
/// wins (a later element overwrites — harmless, matches the linear scan's first-hit at the same position
/// only when names are distinct, which they are for the desugar's unique `__le{pos}` and typical code).
fn build_simple_list_binders(
    db: &Db,
    list_pat: StructId,
) -> Option<std::rc::Rc<crate::db::SimpleListBinders>> {
    let elems = db.ast.compound_form_of(list_pat, CompoundCtor::List)?;
    let marker = db.ast.rest_marker(elems);
    let lead = marker.map(|(i, _, _)| i).unwrap_or(elems.len());
    let mut by_name: crate::fxhash::FxHashMap<String, Vec<crate::core::PathStep>> =
        crate::fxhash::FxHashMap::default();
    for (i, &elem) in elems[..lead].iter().enumerate() {
        #[cfg(test)]
        crate::db::LIST_PATTERN_BINDER_ELEMS_SCANNED.with(|c| c.set(c.get() + 1));
        // Only a BARE name element keeps the pattern simple. A `_`, a nested tuple/variant/list, or any
        // other form disqualifies it: `_` is not a binder (the pattern can still be simple, so continue),
        // while a nested element means the index would have to enumerate deeper paths — bail to the linear
        // descent instead (`None`), which handles those exactly.
        match db.ast.as_name(elem) {
            Some("_") => continue,
            Some(n) => {
                // FIRST-wins, matching the linear scan (which returns the FIRST matching leading position):
                // a repeated name keeps its earliest `Elem`. (Pattern linearity normally rejects a repeat,
                // but preserve the exact verdict regardless.)
                by_name
                    .entry(n.to_string())
                    .or_insert_with(|| vec![crate::core::PathStep::Elem(i)]);
            }
            None => return None, // a nested (tuple/variant/list) element — not simple
        }
    }
    // A trailing `.. rest` binds the tail sublist from `lead` onward (`_` after `..` binds nothing). A
    // LEADING binder of the same name takes precedence (the callers try leading before rest), so only
    // record the rest binder if no leading position already claimed the name (`or_insert`).
    if let Some((_, rest_occ, _)) = marker
        && let Some(rest_name) = db.ast.as_name(rest_occ)
        && rest_name != "_"
        // `!= ".."` guards the helper's malformed-flat fallback (a bare `..` with no operand sibling
        // returns the marker atom itself; `..` is never a real rest-binder name) — byte-identical to the
        // old `elems.get(dd + 1)` None-skip, and harmless for a well-formed operand (a real name).
        && rest_name != ".."
    {
        by_name
            .entry(rest_name.to_string())
            .or_insert_with(|| vec![crate::core::PathStep::RestFrom(lead)]);
    }
    Some(std::rc::Rc::new(crate::db::SimpleListBinders { by_name }))
}

/// The REST binder start-index of a LIST PATTERN `(list p0 … p_{lead-1} .. rest)` binding `name` — the
/// form-independent core shared by the match-arm ([`list_pattern_rest_binds`]) and the `let`/param binding
/// paths. `Some(lead)` (the rest sublist starts at index `lead` = the number of leading positions before
/// `..`) iff `name` is exactly the single binder immediately after `..`; `None` otherwise (or for a
/// fixed-arity pattern with no `..`). `_` is not a binder.
fn find_rest_binder_in_list_pattern(db: &Db, list_pat: StructId, name: &str) -> Option<usize> {
    // FAST PATH: a SIMPLE pattern's rest binder is indexed as a `[RestFrom(lead)]` step (`lead` = its
    // start index), so this is an O(1) map read — the negative-lookup analogue of the leading-binder fast
    // path (a body referencing many OUTER names against a wide `(list … .. rest)` arm would else pay an
    // O(leading) `..`-scan per reference → O(N²)).
    if let Some(binders) = simple_list_binders(db, list_pat) {
        return binders
            .by_name
            .get(name)
            .and_then(|steps| match steps.first() {
                Some(&crate::core::PathStep::RestFrom(lead)) => Some(lead),
                _ => None,
            });
    }
    // SLOW PATH (a nested-element pattern): the exact `..`-position scan.
    let elems = db.ast.compound_form_of(list_pat, CompoundCtor::List)?;
    let (dd, rest_occ, _) = db.ast.rest_marker(elems)?;
    if db.ast.as_name(rest_occ) != Some(name) || name == "_" {
        return None;
    }
    Some(dd)
}

/// If `form` is a match arm `((list p… …) body)` ascended from `body`, and the list pattern binds `name`
/// at a LEADING element position — possibly NESTED inside that element's own sub-pattern — return
/// `(scrutinee, path, heads)`: the access path is `Elem(i)` (into leading position `i` of the list) then
/// the steps the element sub-pattern imposes (a `(tuple a b)` element adds a further `Elem(j)`, a `(Some
/// x)` element adds `[Payload]`), and `heads` the variant heads at each `Payload` step. Handles a bare
/// binder element `(list a b)` (path `[Elem(i)]`), a nested tuple `(list (tuple a b) .. r)` (path
/// `[Elem(i), Elem(j)]`), and a nested irrefutable constructor `(list (Mk n) …)` (path `[Elem(i),
/// Payload]`) — element positions compose exactly as a variant payload / tuple element does
/// (`core-semantics.md §A List Is Deconstructed By Element Patterns With An Optional Rest`, §145). The REST
/// binder (the name after `..`) binds a SUBLIST, matched by `list_pattern_rest_binds`, not here. `None` if
/// no leading element sub-pattern binds `name`.
fn list_pattern_element_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(StructId, Vec<crate::core::PathStep>, Vec<StructId>)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 || pb[1] != from {
        return None; // must be `(pattern body)` ascended from the body
    }
    // Peel a `(guard <list-pattern> <cond>)` wrapper: a GUARDED list arm's pattern is the guard's INNER
    // pattern, so its body binder resolves against that inner `(list …)` exactly as an unguarded arm's does
    // (Case 6lg wires the guard COND's reference; this wires the arm BODY's, ascended from `body`). Without
    // this peel a guarded list arm body's `(list x .. rest)` binder fell through to the global scope →
    // CDZ0101, though the constant-body twin + the guard cond + the unguarded arm body all resolve it.
    let list_pat = match db.ast.as_form(pb[0], "guard") {
        Some(g) if g.len() == 2 => g[0],
        _ => pb[0],
    };
    let (path, heads) = find_leading_binder_in_list_pattern(db, list_pat, name)?;
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    Some((scrutinee, path, heads))
}

/// The REST binder of a `(list p0 … p_{lead-1} .. rest)` match pattern binding `name`: returns
/// `(scrutinee, lead)` — the enclosing match's scrutinee and the number of LEADING binders before `..`
/// (so the rest sublist starts at index `lead`). `None` unless `name` is exactly the binder immediately
/// after a `..` marker in the arm's list PATTERN. The rest sublist companion of `list_pattern_element_binds`.
fn list_pattern_rest_binds(
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
    // Peel a `(guard <list-pattern> <cond>)` wrapper — a guarded list arm's REST binder resolves against
    // the guard's inner `(list …)` in the arm body, the rest-sublist companion of the leading-element peel
    // in `list_pattern_element_binds` (Case 6lg handles the guard COND's rest reference).
    let list_pat = match db.ast.as_form(pb[0], "guard") {
        Some(g) if g.len() == 2 => g[0],
        _ => pb[0],
    };
    let dd = find_rest_binder_in_list_pattern(db, list_pat, name)?;
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    Some((scrutinee, dd)) // `dd` = number of leading binders = the rest sublist's start index
}

/// Whether the `(list …)` FORM `pat` has a NON-NAME rest slot — a `..` marker followed by EXACTLY one
/// element that is NOT a bare name / `_`: a nested `(list …)` / `(tuple …)` / `(Mk …)` sub-pattern OR a
/// literal (`(list a .. 5)`). A list rest binder admits only a name or wildcard (RULED v-inference
/// 2026-08-02: core-semantics.md:149/:135 — a binding position holds an irrefutable pattern, and a nested
/// list pattern is refutable on empty rest; a literal binds nothing), so ANY non-name/`_` rest is invalid;
/// its inner binders (if any) must resolve to the rest-shape decline (Case 6mr) rather than leaking a
/// misleading unbound name. The list twin of [`map_form_is_malformed_rest`]. A well-formed name/`_` rest,
/// or a `..` not followed by exactly one element (a different malformed shape the lowering arity check
/// owns), returns `false`. (The predicate keys on `as_name(...).is_none()`, so it covers literals as well
/// as compound sub-patterns — the comment and the message both say "name or `_`", not "nested", to match.)
fn list_form_has_nested_rest(db: &Db, pat: StructId) -> bool {
    let Some(elems) = db.ast.compound_form_of(pat, CompoundCtor::List) else {
        return false;
    };
    match db.ast.rest_marker(elems) {
        // The rest is the LAST element (nothing trailing after it) and its operand is NOT a bare name /
        // `_` (a compound sub-pattern OR a literal) — the invalid non-name rest shape. `trailing_start ==
        // len` covers both marker shapes (flat `.. x` at the end → `i + 2 == len`; wrapped `(.. x)` at the
        // end → `i + 1 == len`); the operand comes from the helper regardless of shape.
        Some((_, operand, trailing_start)) if trailing_start == elems.len() => {
            db.ast.as_name(operand).is_none()
        }
        _ => false,
    }
}

/// Whether the NESTED-REST slot of `pat` (a `(list …)` form with [`list_form_has_nested_rest`]) binds
/// `name` ANYWHERE inside its rest sub-pattern — a leading element (`b` in `(list b .. r)`), the sub-
/// pattern's own rest binder (`r`), or deeper. Uses the form-independent `find_binder_in_*` walkers (the
/// same ones the body-resolution cases use), so it recognizes exactly the names a body reference would
/// otherwise resolve unbound. `name`/`_`/`..` bind nothing. Companion of [`list_form_has_nested_rest`].
fn nested_rest_slot_binds_name(db: &Db, pat: StructId, name: &str) -> bool {
    if name == "_" || name == ".." {
        return false;
    }
    let Some(elems) = db.ast.compound_form_of(pat, CompoundCtor::List) else {
        return false;
    };
    let Some((_, rest_pat, _)) = db.ast.rest_marker(elems) else {
        return false;
    };
    let (mut path, mut heads) = (Vec::new(), Vec::new());
    if is_tuple_pattern(db, rest_pat) {
        find_binder_in_tuple(db, rest_pat, name, &mut path, &mut heads)
    } else if is_list_pattern(db, rest_pat) {
        find_binder_in_list(db, rest_pat, name, &mut path, &mut heads)
    } else {
        find_binder_in_pattern(db, rest_pat, name, &mut path, &mut heads)
    }
}

/// The `(list …)` PATTERN node when `form` is a MATCH ARM `(pattern body)` (ascended from its BODY, or a
/// guarded arm's guard cond) whose pattern is — DIRECTLY or NESTED inside a tuple/variant/list payload — a
/// `(list …)` with a NESTED-PATTERN rest slot ([`list_form_has_nested_rest`]) that binds `name`; `None`
/// otherwise. The list twin of [`match_arm_malformed_map_binds`]: it lets `binder_in` (Case 6mr) resolve
/// such a body/guard reference to the clear rest-shape decline (co-anchored at the returned node) instead
/// of leaking an unbound name.
fn match_arm_nested_list_rest_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<StructId> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    // Peel a `(guard <pattern> <cond>)` wrapper — a guard-cond reference binds the same names.
    let (arm_pat, guard_cond) = match db.ast.as_form(pb[0], "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pb[0], None),
    };
    if from != pb[1] && Some(from) != guard_cond {
        return None;
    }
    // Must be a genuine match arm (parent is `(match scrutinee arm…)`, `form` an arm, not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    match mtail.first() {
        Some(&scrutinee) if scrutinee != form => {}
        _ => return None,
    }
    find_nested_list_rest_binding_name(db, arm_pat, name)
}

/// The NESTED-REST `(list …)` sub-pattern of `pattern` that binds `name` in its rest slot, searched
/// DIRECTLY and through nested tuple/variant/list payloads; `None` if none. Companion of
/// [`match_arm_nested_list_rest_binds`] — lets a body reference to a nested-rest inner binder resolve to
/// the rest-shape decline (not a leaked unbound name), including when the offending `(list …)` is itself
/// nested inside another compound pattern.
fn find_nested_list_rest_binding_name(db: &Db, pattern: StructId, name: &str) -> Option<StructId> {
    // A `(list …)` here whose rest slot is a nested pattern AND binds `name`: this is the node.
    if list_form_has_nested_rest(db, pattern) && nested_rest_slot_binds_name(db, pattern, name) {
        return Some(pattern);
    }
    // Descend a compound pattern's children (including the invalid nested-rest sub-pattern itself, so a
    // doubly-nested rest is caught). Bounded by the arena's tree depth.
    let Struct::List(children) = db.ast.get(pattern) else {
        return None;
    };
    for &child in children {
        if let Some(found) = find_nested_list_rest_binding_name(db, child, name) {
            return Some(found);
        }
    }
    None
}

/// Whether `id` is a NAME occurrence that is a `(list …)` match-pattern binder in the PATTERN position — a
/// LEADING element binder, the `..` marker, OR the rest binder — all of which must resolve INERT (not a
/// looked-up value) so walking the arm's pattern never reports them unbound. (A body reference to a
/// leading binder resolves to `SumPayload` via Case 6l; the rest binder is inert until a used-sublist
/// increment.) Mirrors the arm/scrutinee shape `list_pattern_element_binds` requires.
/// Whether `id` is a bare-NAME binder occurrence inside an arm's VARIANT/TUPLE PATTERN — the `n` in
/// `((W.V n) …)`, `((W.V (Option.Some n)) …)`, or a tuple-payload `((P.Mk a b) …)`. Such an occurrence
/// NAMES A BINDING, not a value, so it must resolve INERT (like a list/map pattern binder) — otherwise an
/// eager subtree walk resolves it via `resolve_name` and, if it shadows an enclosing param, binds it to
/// that param, corrupting the pattern. Ascends to the enclosing arm's PATTERN (the first element of a
/// 2-element arm under a `(match …)`), then confirms `id` is reached as a payload BINDER by walking the
/// pattern with `find_binder_in_pattern`/`find_binder_in_tuple` (the same walkers `binder_in` Case 6 uses),
/// so it agrees exactly with where a body reference would resolve. A pattern HEAD (`(. W V)`'s parts, a
/// bare variant name), a literal, `_`, or a KEY/value position of a list/map pattern (their own inert
/// cases handle those) is NOT reported here.
fn is_variant_pattern_binder_occurrence(db: &Db, id: StructId) -> bool {
    // `id` must be a bare name (not `_`). Find the enclosing ARM PATTERN: ascend until a node that is the
    // FIRST element of a 2-element arm under a `(match …)`. Bounded by a small hop count (patterns are
    // shallow); a reference in an arm BODY never ascends through the pattern, so this only fires in-pattern.
    let Some(nm) = db.ast.as_name(id) else {
        return false;
    };
    if nm == "_" {
        return false;
    }
    // Walk up from `id` to the arm's pattern node (the child of the arm that the pattern subtree roots at).
    // `prev` trails one step behind `node` — the child of `node` the ascent came up through — so when
    // `node` turns out to be a guard we can tell in O(1) which of its children (`g[0]` inner pattern vs
    // `g[1]` cond) holds `id`, without re-searching the (possibly O(N)-deep) cond subtree.
    let mut node = id;
    let mut prev = id;
    let mut hops = 0;
    while hops < 64 {
        let Some(parent) = db.parent_of(node) else {
            return false;
        };
        // Is `parent` a match arm `(pattern body)` with `node` as its PATTERN (first element)?
        if let Struct::List(pb) = db.ast.get(parent)
            && pb.len() == 2
            && pb[0] == node
            && let Some(gp) = db.parent_of(parent)
            && let Some(mtail) = db.ast.as_form(gp, "match")
            && mtail.first().copied() != Some(parent)
            && mtail.contains(&parent)
        {
            // `node` is the arm's pattern. A GUARD wrapper `(guard <inner-pattern> <cond>)` binds via its
            // INNER pattern — but the guard COND is a VALUE expression that READS the binders (`(> n 5)`),
            // so a reference reached from the COND must resolve NORMALLY (Case 5g/6 gives it the payload),
            // NOT inert. Only treat `id` inert if it sits in the INNER PATTERN, not the cond: unwrap the
            // guard and require the ascent to have come UP THROUGH the inner pattern (i.e. `id` is within
            // `g[0]`, not `g[1]`).
            let pattern = match db.ast.as_form(node, "guard") {
                Some(g) if g.len() == 2 => {
                    // `prev` is the DIRECT child of `node` (the guard) the ascent passed through, so `id` is
                    // in the cond iff `prev == g[1]` — an O(1) check. (A guard's children are `[guard, g0,
                    // g1]`; `id` is a name nested below, so `prev` is `g[0]` or `g[1]`.) This replaces an
                    // O(cond-size) `node_contains(g[1], id)` re-descent — the O(N²) driver on a desugared
                    // refutable-literal arm whose cond is an O(N)-deep `and`-chain (N binders × O(N) scan).
                    if prev == g[1] {
                        return false; // `id` is in the guard COND — a value reference, resolve normally.
                    }
                    g[0]
                }
                _ => node,
            };
            // Confirm `id`'s NAME is bound as a payload binder in the pattern — the same walk `binder_in`
            // Case 6 uses. `id` is known to sit in the pattern subtree (not the body — a body reference
            // ascends from `pb[1]`, never making `node == pb[0]`), so a name match IS this occurrence.
            let mut path = Vec::new();
            let mut heads = Vec::new();
            let binds = find_binder_in_pattern(db, pattern, nm, &mut path, &mut heads)
                || (is_tuple_pattern(db, pattern) && {
                    let mut p = Vec::new();
                    let mut h = Vec::new();
                    find_binder_in_tuple(db, pattern, nm, &mut p, &mut h)
                })
                // A TOP-LEVEL RECORD pattern `(record (x a) (y b))` binds its field VALUES (`a`/`b`) — a
                // field-value binder in the pattern position must inert exactly like a tuple element binder,
                // else the eager subtree walk mis-binds a shadowing field binder to an outer param. Its own
                // resolution (Case 6rec → `Member`) is a projection of the scrutinee, so the pattern
                // occurrence carries no value. (`find_binder_in_pattern` excludes the `record` compound
                // head, so the record shape needs this explicit check.)
                || (db
                    .ast
                    .compound_form_of(pattern, crate::ast::CompoundCtor::Record)
                    .is_some()
                    && record_pattern_binds_name(db, pattern, nm));
            if !binds {
                return false;
            }
            // Fire the INERT classification ONLY when the name would OTHERWISE resolve to an OUTER binding
            // (a genuine SHADOW) — the exact bug condition. A NON-shadowing payload binder keeps its prior
            // resolution (it never fell to an outer name, so the eager walk was harmless + the constant-fold
            // path that maps the binder to its payload value stays intact). `lookup_scope` from the ENCLOSING
            // form (the arm's parent — skip the arm itself so we see only OUTER binders, not this pattern)
            // finding a binder for `nm` means shadowing. This preserves every prior case and inerts only the
            // shadowing pattern binder the `resolve_subtree` walk would misbind to the outer param.
            let shadows = db
                .parent_of(parent)
                .and_then(|outer| lookup_scope(db, outer, nm))
                .is_some();
            return shadows;
        }
        prev = node;
        node = parent;
        hops += 1;
    }
    false
}

fn is_list_pattern_element_occurrence(db: &Db, id: StructId) -> bool {
    // `id` must be a bare name (not `_`, not the `..` marker). A binder occurrence may be a DIRECT child
    // of the `(list …)` pattern (`(list a b)` — `a`/`b`) or NESTED inside a leading element sub-pattern
    // (`(list (tuple a b) .. r)` — `a`/`b` inside the tuple), so we ascend to the enclosing `(list …)`
    // arm pattern rather than only checking the direct parent.
    let Some(nm) = db.ast.as_name(id) else {
        return false;
    };
    if nm == "_" || nm == ".." {
        return false;
    }
    // Ascend from `id` to the enclosing `(list …)` form that is an arm's PATTERN — directly `((list …)
    // body)` OR under a guard `((guard (list …) cond) body)`. Bounded (patterns are shallow). A reference
    // in the arm BODY ascends from the body, never through the pattern, so this only fires for an
    // occurrence sitting inside the pattern subtree.
    let mut node = id;
    let mut hops = 0;
    while hops < 64 {
        let Some(parent) = db.parent_of(node) else {
            return false;
        };
        // Is `parent` a `(list …)` pattern whose enclosing form is a match arm — directly, or wrapped in a
        // `(guard <list> cond)` (whose grandparent is then the arm)?
        if db
            .ast
            .compound_form_of(parent, crate::ast::CompoundCtor::List)
            .is_some()
            && list_pattern_is_arm_pattern(db, parent)
        {
            // `parent` is the arm's list pattern. `id` is a genuine binder iff a leading element sub-pattern
            // or the rest binder binds its name — the SAME form-independent walk Cases 6l/6r/6lg use, so it
            // agrees exactly with where a body/guard reference resolves. (A pattern HEAD — the `(. Sum V)`
            // parts, a bare variant name — is not a binder; the walkers skip heads.)
            return find_leading_binder_in_list_pattern(db, parent, nm).is_some()
                || find_rest_binder_in_list_pattern(db, parent, nm).is_some();
        }
        node = parent;
        hops += 1;
    }
    false
}

/// Whether the `(list …)` node `list_pat` is a match ARM's pattern — either directly `((list …) body)` or
/// under a guard wrapper `((guard (list …) cond) body)` — of a `(match scrutinee arm…)`. Shared by the
/// inert-binder classifier (which fires for a binder in the pattern position of both shapes).
fn list_pattern_is_arm_pattern(db: &Db, list_pat: StructId) -> bool {
    // The pattern node the arm holds is `list_pat` itself, or the `(guard <list_pat> cond)` wrapping it.
    let arm_pat = match db.parent_of(list_pat) {
        Some(p) if matches!(db.ast.as_form(p, "guard"), Some(g) if g.len() == 2 && g[0] == list_pat) => {
            p
        }
        _ => list_pat,
    };
    let Some(arm) = db.parent_of(arm_pat) else {
        return false;
    };
    let Struct::List(pb) = db.ast.get(arm) else {
        return false;
    };
    if pb.len() != 2 || pb[0] != arm_pat {
        return false;
    }
    match db.parent_of(arm) {
        Some(matchf) => matches!(db.ast.as_form(matchf, "match"),
            Some(mtail) if mtail.first().copied() != Some(arm) && mtail.contains(&arm)),
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
    let tail = db.ast.compound_form_of(pat, CompoundCtor::Map)?;
    // Split at a `..` marker: the entries before it, then exactly one rest binder after.
    let (entries_tail, rest) = match db.ast.rest_marker(tail) {
        Some((i, operand, trailing_start)) => {
            if trailing_start != tail.len() {
                return None; // `..` must be followed by exactly one rest binder
            }
            (&tail[..i], Some(operand))
        }
        None => (tail, None),
    };
    let mut entries = Vec::with_capacity(entries_tail.len());
    for &entry in entries_tail {
        // A map-pattern entry is the native `(= k p)` FieldPair leaf (M2, the `#map((= k p))` surface), the
        // transitional name-head `(= k p)`, or the legacy 2-element `(k p)` pair — mirror `resolve_map`. Before
        // reading the FieldPair, a native `#map` pattern's `(= k p)` entry (3-element) failed the 2-element
        // check → `map_pattern_of` returned None → the pattern's native ctor-leaf head leaked to value
        // resolution (CDZ0201 "compound-constructor head leaf is not a value").
        if let Some((k, v)) = db
            .ast
            .field_pair_parts(entry)
            .or_else(|| db.ast.field_pair(entry))
        {
            entries.push((k, v));
            continue;
        }
        match db.ast.get(entry) {
            Struct::List(items) if items.len() == 2 => entries.push((items[0], items[1])),
            _ => return None,
        }
    }
    Some((entries, rest))
}

/// Whether `pat` is a `(map …)` FORM whose `..` rest is MALFORMED — a `..` marker NOT followed by exactly
/// one binder (`(map (k v) .. rest (j w))` — non-final; `(map (k v) .. r1 .. r2)` — two). `map_pattern_of`
/// collapses this to `None` (indistinguishable from "not a map form at all"), which made a malformed-rest
/// arm's value/rest binders fail the inert-binder classifier → the body reference resolved UNBOUND, a
/// misleading CDZ0101 masking the real rest-shape fault (v-diagnostics note 2026-07-16). This predicate
/// re-detects that specific case so the map matcher can emit the SAME clear rest-shape message the list
/// matcher gives (`lower_match_list` / `check_binding_pattern`) instead of the generic "not a map pattern",
/// and so [`is_map_pattern_binder_occurrence`] can still classify the binders inert (suppressing the
/// unbound cascade — the map twin of how the list rest-shape check keeps its binders inert while faulting).
pub(crate) fn map_form_is_malformed_rest(db: &Db, pat: StructId) -> bool {
    let Some(tail) = db.ast.compound_form_of(pat, CompoundCtor::Map) else {
        return false;
    };
    match db.ast.rest_marker(tail) {
        // A `..` present but not followed by EXACTLY one trailing binder — the malformed rest shape.
        Some((_, _, trailing_start)) => trailing_start != tail.len(),
        None => false,
    }
}

/// Whether the `(map …)` FORM `pat` — WELL-FORMED OR MALFORMED — binds `name` at a value or rest position
/// (a KEY position is never a binder). Unlike [`map_pattern_of`] (which returns `None` on a malformed
/// `..`), this scans the tail STRUCTURALLY, so a malformed-rest arm's binders are still recognized — the
/// map twin of `find_leading_binder_in_list_pattern`/`find_rest_binder_in_list_pattern`, which position-scan
/// a list pattern regardless of its overall shape. Used by [`is_map_pattern_binder_occurrence`] to keep a
/// malformed-rest arm's binders INERT so a body reference does not resolve UNBOUND before the rest-shape
/// fault is reported.
fn map_form_binds_name(db: &Db, pat: StructId, name: &str) -> bool {
    if name == "_" || name == ".." {
        return false;
    }
    let Some(tail) = db.ast.compound_form_of(pat, CompoundCtor::Map) else {
        return false;
    };
    let marker = db.ast.rest_marker(tail);
    let dotdot = marker.map(|(d, _, _)| d);
    // Whether a map ENTRY binds `name` at its VALUE position (the key is a label, never a binder). Accepts
    // the native `(= k v)` FieldPair (M2 `#map((= k v))` — a FieldPair leaf or 3-element list) AND the legacy
    // 2-element `(k v)` pair — mirroring `map_pattern_of`'s entry read. Without the FieldPair arm a native
    // `#map` value binder failed the 2-element check → not recognized inert → the malformed-two-`..` case
    // leaked a spurious CDZ0101 on the value binder (v-rcdzc-test-shrink report).
    let entry_binds_value = |item: StructId| -> bool {
        if let Some((_, v)) = db
            .ast
            .field_pair_parts(item)
            .or_else(|| db.ast.field_pair(item))
        {
            return db.ast.as_name(v) == Some(name);
        }
        matches!(db.ast.get(item), Struct::List(kv) if kv.len() == 2 && db.ast.as_name(kv[1]) == Some(name))
    };
    for (i, &item) in tail.iter().enumerate() {
        match dotdot {
            // The `..` marker itself binds nothing — EXCEPT the wrapped `(.. rest)` node CARRIES its rest
            // binder as the operand inside it (the flat form puts it at `d + 1`, handled by the `i > d`
            // arm), so check that operand here.
            Some(d) if i == d => {
                if let Some((_, operand, _)) = marker
                    && db.ast.as_form(item, "..").is_some()
                    && db.ast.as_name(operand) == Some(name)
                {
                    return true;
                }
            }
            // Item(s) AFTER a `..` in a MALFORMED pattern. In a well-formed pattern this is exactly one
            // bare rest binder, but a malformed one may put another `(k v)` ENTRY here (`(map (1 v) ..
            // rest (2 w))`) — treat BOTH shapes as binding: a bare name is a rest-position binder, and a
            // `(k v)` pair binds its VALUE `v` (the key is a label). Without the `(k v)` arm, a body
            // reference to a value binder after `..` (`w`) leaked a misleading CDZ0101 on top of the
            // already-malformed pattern (Copilot PR #440 / corpus-bugfix). The KEY is never a binder.
            Some(d) if i > d => {
                if db.ast.as_name(item) == Some(name) {
                    return true;
                }
                if entry_binds_value(item) {
                    return true;
                }
            }
            // An entry `(key value)`/`(= key value)` BEFORE `..` (or in a pattern with no `..`) — the VALUE
            // is the binder; the KEY is not.
            _ => {
                if entry_binds_value(item) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether `pat` is a `(list …)` FORM whose `..` rest is MALFORMED — a `..` marker NOT followed by exactly
/// one binder (`(list a .. b c)` — extra binders after the rest; `(list a .. b .. c)` — two `..`). The list
/// twin of [`map_form_is_malformed_rest`]. `lower_match_list` already emits the clear rest-shape CDZ0201 at
/// the pattern, but a body reference to one of the EXTRA post-`..` binders (`c` in `(list a .. b c)`) fell
/// through to `resolve_name` → a misleading `CDZ0101 unbound c` cascade on top of the real fault — the same
/// class the map twin fixed (v-diagnostics note 2026-07-16). This predicate re-detects the shape so
/// [`match_arm_malformed_list_binds`] can resolve such a reference to the SAME coded rest-shape decline,
/// suppressing the cascade. (`find_rest_binder_in_list_pattern` recognizes ONLY the single binder at
/// `dd + 1`, so the extras are neither a valid binder nor otherwise classified.)
fn list_form_is_malformed_rest(db: &Db, pat: StructId) -> bool {
    let Some(elems) = db.ast.compound_form_of(pat, CompoundCtor::List) else {
        return false;
    };
    match db.ast.rest_marker(elems) {
        // A rest present but with elements AFTER it (flat: not exactly one operand sibling; wrapped: extra
        // elements past the `(.. x)` node) — the malformed rest shape. `trailing_start != len` unifies both
        // shapes (flat well-formed rest has `i + 2 == len`, wrapped has `i + 1 == len`).
        Some((_, _, trailing_start)) => trailing_start != elems.len(),
        None => false,
    }
}

/// Whether the MALFORMED-rest `(list …)` FORM `pat` binds `name` at one of the EXTRA positions AFTER the
/// `..` (position `dd + 2` onward) — the binders `find_rest_binder_in_list_pattern` (which only recognizes
/// the single `dd + 1` rest binder) leaves unclassified, so a body reference to them leaked unbound. Only
/// bare-name items are counted (`_` binds nothing; a nested sub-pattern after `..` is a separate decline).
/// The `dd + 1` position is deliberately EXCLUDED — it is the legitimate rest binder, already resolved by
/// Case 6r / kept inert by `is_list_pattern_element_occurrence`; this covers strictly the surplus.
fn list_form_binds_post_rest_name(db: &Db, pat: StructId, name: &str) -> bool {
    if name == "_" || name == ".." {
        return false;
    }
    let Some(elems) = db.ast.compound_form_of(pat, CompoundCtor::List) else {
        return false;
    };
    let Some((_, _, trailing_start)) = db.ast.rest_marker(elems) else {
        return false;
    };
    // The surplus positions are those AFTER the rest (`trailing_start ..`); the rest binder itself (the
    // operand — flat `dd + 1`, wrapped inside the `(.. x)` node) is the legitimate one, handled elsewhere.
    elems
        .iter()
        .skip(trailing_start)
        .any(|&e| db.ast.as_name(e) == Some(name))
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
        // Is `cand` WITHIN a match arm's PATTERN? Directly (the map IS the arm pattern — Case M) or NESTED
        // inside a tuple/record/variant arm pattern (the map is a sub-pattern — Case Mn). Ascend from
        // `cand` through its enclosing compound patterns to the arm's pattern slot; a binder occurrence in
        // EITHER position names a binding, not a value, so it must resolve inert.
        if !map_pattern_is_in_a_match_arm(db, cand) {
            continue;
        }
        if let Some((entries, rest)) = map_pattern_of(db, cand) {
            // `id` is a binder iff it is a VALUE position of some entry, or the REST binder. (A KEY
            // position is NOT — it resolves as a value.)
            if rest == Some(id) || entries.iter().any(|&(_, v)| v == id) {
                return true;
            }
        } else if map_form_is_malformed_rest(db, cand) {
            // A `(map …)` form whose `..` rest is MALFORMED (`map_pattern_of` gave `None`). Its
            // value/rest binders must STILL classify inert — else the arm body's reference to one resolves
            // UNBOUND, masking the real rest-shape fault (v-diagnostics note). Scan the tail structurally
            // by NAME (the map twin of the list rest-shape leniency); `id`'s name at a binder position
            // makes it inert. The clear rest-shape CDZ0201 is then the sole diagnostic (emitted by the map
            // matcher / the binding path), not a phantom unbound name.
            if let Some(nm) = db.ast.as_name(id)
                && map_form_binds_name(db, cand, nm)
            {
                return true;
            }
        }
    }
    false
}

/// Whether the map pattern `cand` sits inside a match arm's PATTERN slot — directly (it IS the pattern) or
/// nested inside the arm's tuple/record/variant/list pattern. Ascends from `cand` to the enclosing arm's
/// first child (the pattern), bounded so it stops at the arm boundary (a 2-element list under a `(match
/// …)` whose first child is on the ascent path). Used to mark a nested map pattern's value/rest binder
/// inert (like a direct one), so an eager subtree walk never reports it a spurious unbound name.
fn map_pattern_is_in_a_match_arm(db: &Db, cand: StructId) -> bool {
    // Walk up: at each step, if the current node is the FIRST child (pattern slot) of a 2-element arm under
    // a `(match …)`, this is a match-arm pattern → true. Bounded by a small depth (patterns don't nest
    // deeply) to avoid runaway on a malformed cycle-free arena.
    let mut node = cand;
    for _ in 0..64 {
        let Some(parent) = db.parent_of(node) else {
            return false;
        };
        if let Struct::List(pb) = db.ast.get(parent)
            && pb.len() == 2
            && pb[0] == node
            && let Some(matchf) = db.parent_of(parent)
            && matches!(db.ast.as_form(matchf, "match"),
                Some(mtail) if mtail.first().copied() != Some(parent) && mtail.contains(&parent))
        {
            return true;
        }
        node = parent;
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
    // When `pattern` ITSELF is a `(. Sum V)` member form, its `app[0]` is the bare `.` ATOM and the
    // remaining children are `Sum` and `V` — the qualified nullary-variant CONSTRUCTOR, NOT a ctor with
    // payload binders `Sum`/`V`. Treating the `.` atom as a variant head (below) would spuriously register
    // `Sum` and `V` as payload binders, poisoning scope: a later IDENTICAL nested `(. Sum V)` pattern then
    // sees `Sum`/`V` as SHADOWED names and resolves them INERT (`Resolved::Unit`), so its member operand no
    // longer reduces to the type record and the nested nullary-variant pattern fails as "member access
    // requires a record". A whole `(. Sum V)` pattern binds NOTHING (a nullary ctor) — bail out. (The other
    // walkers, e.g. `lower::pattern_constraints`, already special-case a `.`-atom head this same way.)
    if db.ast.as_name(head) == Some(".") {
        return false;
    }
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
            } else if is_list_pattern(db, arg) {
                find_binder_in_list(db, arg, name, path, heads)
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
    if is_list_pattern(db, arg) {
        return find_binder_in_list(db, arg, name, path, heads);
    }
    find_binder_in_pattern(db, arg, name, path, heads)
}

/// Descend a LIST pattern `(list p0 p1…)` (a variant's list payload) looking for the element binder
/// `name`, appending its access-path steps. The list analogue of [`find_binder_in_tuple`] — each element
/// binds at `Elem(i)` from the list handle (the SAME step `const_at_path`'s `ListNew` arm reads), so a
/// `(Ast.List (list (Ast.Name "+") a b))` quote pattern binds `a`/`b` at `Payload, Elem(1)`/`Elem(2)`.
/// A rest binder `.. rest` is NOT handled here (a runtime sublist — declines in `pattern_constraints`).
fn find_binder_in_list(
    db: &Db,
    pattern: StructId,
    name: &str,
    path: &mut Vec<crate::core::PathStep>,
    heads: &mut Vec<StructId>,
) -> bool {
    let raw: Vec<StructId> = db
        .ast
        .compound_form_of(pattern, CompoundCtor::List)
        .unwrap_or(&[])
        .to_vec();
    // Split off a trailing `.. rest`: the rest binder (the single element after the `..` marker) binds the
    // TAIL SUBLIST from index `lead` onward, via a `RestFrom(lead)` step (`const_at_path`'s `RestFrom`/
    // `ListNew` fold; the payload analogue of the top-level `list_rest_binder` path). `lead` = the leading
    // fixed element patterns.
    let (leads, rest): (&[StructId], Option<StructId>) = match db.ast.rest_marker(&raw) {
        Some((k, operand, trailing_start)) if trailing_start == raw.len() => {
            (&raw[..k], Some(operand))
        }
        _ => (&raw[..], None),
    };
    if let Some(rest) = rest
        && db.ast.as_name(rest) == Some(name)
        && name != "_"
    {
        path.push(crate::core::PathStep::RestFrom(leads.len()));
        return true;
    }
    for (i, &elem) in leads.iter().enumerate() {
        let path_len = path.len();
        let heads_len = heads.len();
        path.push(crate::core::PathStep::Elem(i));
        let found = if let Some(elem_name) = db.ast.as_name(elem) {
            elem_name == name && elem_name != "_"
        } else if is_tuple_pattern(db, elem) {
            find_binder_in_tuple(db, elem, name, path, heads)
        } else if is_list_pattern(db, elem) {
            find_binder_in_list(db, elem, name, path, heads)
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

/// Whether `id` is a tuple PATTERN `(tuple p0 p1…)` — a `tuple` NAME head (the shadowable alias the
/// reader keeps in a pattern) or the `"tuple"` string-literal primitive. Used to route a variant's
/// tuple payload into element-by-element descent.
fn is_tuple_pattern(db: &Db, id: StructId) -> bool {
    // Recognize the native ctor-leaf head (M2 `#tuple(…)` pattern) as well as the name/string
    // heads — `compound_form_of` covers all three.
    db.ast.compound_form_of(id, CompoundCtor::Tuple).is_some()
}

/// Whether a `(record (field value) …)` PATTERN binds `name` anywhere — at a bare field value `(x a)` or
/// NESTED inside a field's compound value `(p (tuple a b))`. Used only to ATTRIBUTE a body reference to an
/// unwired Increment-B nested record sub-pattern (a coded decline), so a partial-implementation reference
/// names the feature rather than falling to a misleading CDZ0101. Recurses into nested tuple/list/record/
/// ctor field values via the same form-independent walkers the wired cases use.
fn record_pattern_binds_name(db: &Db, record_pat: StructId, name: &str) -> bool {
    let Some(fields) = db
        .ast
        .compound_form_of(record_pat, CompoundCtor::Record)
        .map(<[_]>::to_vec)
    else {
        return false;
    };
    fields.iter().any(|&pair| {
        // A record-pattern field is the canonical `(= key sub-pattern)` triple (Phase B) — or a legacy
        // `(key sub-pattern)` pair; `record_pattern_field_kv` returns the (key, sub-pattern) for both.
        let Some((_key_id, value_pat)) = record_pattern_field_kv(db, pair) else {
            return false;
        };
        if let Some(nm) = db.ast.as_name(value_pat) {
            return nm == name && nm != "_";
        }
        let (mut path, mut heads) = (Vec::new(), Vec::new());
        if is_tuple_pattern(db, value_pat) {
            find_binder_in_tuple(db, value_pat, name, &mut path, &mut heads)
        } else if is_list_pattern(db, value_pat) {
            find_binder_in_list(db, value_pat, name, &mut path, &mut heads)
        } else if db
            .ast
            .compound_form_of(value_pat, crate::ast::CompoundCtor::Record)
            .is_some()
        {
            record_pattern_binds_name(db, value_pat, name)
        } else {
            find_binder_in_pattern(db, value_pat, name, &mut path, &mut heads)
        }
    })
}

/// Whether `id` is a list PATTERN `(list p0 p1…)` — a `list` NAME head (the shadowable alias) or the
/// `"list"` string-literal primitive. Routes a variant's list payload into element-by-element binder
/// descent ([`find_binder_in_list`]), the list analogue of [`is_tuple_pattern`].
fn is_list_pattern(db: &Db, id: StructId) -> bool {
    // Recognize the native ctor-leaf head (M2 `#list(…)` pattern) as well as the name/string
    // heads — `compound_form_of` covers all three.
    db.ast.compound_form_of(id, CompoundCtor::List).is_some()
}

/// Whether `id` is a map PATTERN `(map (k v) … .. rest)` — a `map` NAME head (the shadowable alias) or the
/// `"map"` string-literal primitive. Routes a NESTED map sub-pattern into [`find_map_binder_in_pattern`]
/// (the key-directed binder descent), the map analogue of [`is_tuple_pattern`]/[`is_list_pattern`].
fn is_map_pattern(db: &Db, id: StructId) -> bool {
    // Recognize the native ctor-leaf head (M2 `#map(…)` pattern) as well as the name/string
    // heads — `compound_form_of` covers all three.
    db.ast.compound_form_of(id, CompoundCtor::Map).is_some()
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
    let raw: Vec<StructId> = db
        .ast
        .compound_form_of(pattern, CompoundCtor::Tuple)
        .unwrap_or(&[])
        .to_vec();
    // Split off a trailing `.. rest`: the rest binder binds the TRAILING SUB-TUPLE from index `lead`
    // onward via a `TupleRestFrom(lead)` step (the tuple analogue of the list `RestFrom` above; a tuple's
    // fixed arity means the sub-tuple is a fixed gather, not a runtime slice). `lead` = the leading fixed
    // element patterns before the `..`.
    let (elems, rest): (&[StructId], Option<StructId>) = match db.ast.rest_marker(&raw) {
        Some((k, operand, trailing_start)) if trailing_start == raw.len() => {
            (&raw[..k], Some(operand))
        }
        _ => (&raw[..], None),
    };
    if let Some(rest) = rest
        && db.ast.as_name(rest) == Some(name)
        && name != "_"
    {
        path.push(crate::core::PathStep::TupleRestFrom(elems.len()));
        return true;
    }
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
        } else if is_list_pattern(db, elem) {
            find_binder_in_list(db, elem, name, path, heads)
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

/// The resolution of a NESTED map-pattern binder: the enclosing match's `scrutinee`, the access `path`
/// from it down to the nested map, `Some(key)` for a VALUE binder at that key (else `None` for the map's
/// REST binder), and the `named` keys the map pattern names.
type NestedMapBind = (
    StructId,
    Vec<crate::core::PathStep>,
    Option<StructId>,
    Vec<StructId>,
    Vec<crate::core::PathStep>, // value_steps: sub-path INTO the value at the key
    Vec<StructId>,              // value_heads: variant heads at each Payload step in value_steps
);

/// If `form` is a match ARM `(pattern body)` (ascended from its BODY) whose pattern is a COMPOUND
/// (tuple/list/variant/record) with a MAP pattern NESTED inside it binding `name`, return the
/// [`NestedMapBind`]. `None` otherwise. The NESTED companion of [`match_arm_map_binds`] (which handles the
/// map as the DIRECT scrutinee).
fn match_arm_nested_map_binds(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<NestedMapBind> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 || from != pb[1] {
        return None; // not an arm, or the reference is not from the arm's body
    }
    let pattern = pb[0];
    // `form`'s parent must be a `(match scrutinee arm…)` and `form` an arm (not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    // The map pattern must be NESTED — a DIRECT map scrutinee is Case M, not here (an empty path would
    // duplicate it). Descend the compound for a map sub-pattern binding `name`.
    let mut path = Vec::new();
    let (key, named, value_steps, value_heads) =
        find_map_binder_in_pattern(db, pattern, name, &mut path)?;
    if path.is_empty() {
        return None; // the map is the whole pattern → Case M's job
    }
    Some((scrutinee, path, key, named, value_steps, value_heads))
}

/// Descend a COMPOUND pattern (tuple/list/variant/record) looking for a MAP sub-pattern that binds `name`,
/// accumulating the `Elem`/`Payload` access steps to reach that map. On a hit returns `Some((key, named))`
/// — `Some(key)` if `name` is the VALUE binder at that key, `None` if it is the map's REST binder — with
/// `path` holding the steps from the enclosing pattern's scrutinee to the map. The map analogue of the
/// element-by-element binder descent (`find_binder_in_tuple`/`_list`/`_pattern`), which handle tuple/list/
/// variant sub-patterns but had no map arm — so a `(map …)` nested in a tuple/record/variant dropped its
/// binder (a spurious "unbound name"). Element positions compose to any depth (`core-semantics.md §145`).
#[allow(clippy::type_complexity)]
fn find_map_binder_in_pattern(
    db: &Db,
    pattern: StructId,
    name: &str,
    path: &mut Vec<crate::core::PathStep>,
) -> Option<(
    Option<StructId>,
    Vec<StructId>,
    Vec<crate::core::PathStep>,
    Vec<StructId>,
)> {
    // A MAP pattern here: does it bind `name` (a value binder at some key — possibly nested in a value
    // sub-pattern — or the rest binder)?
    if is_map_pattern(db, pattern) {
        let (entries, rest) = map_pattern_of(db, pattern)?;
        let named: Vec<StructId> = entries.iter().map(|&(k, _)| k).collect();
        for &(k, v) in &entries {
            if let Some((vsteps, vheads)) = value_subpattern_binds(db, v, name) {
                return Some((Some(k), named, vsteps, vheads)); // a VALUE binder at key `k`
            }
        }
        if rest.is_some_and(|r| db.ast.as_name(r).is_some_and(|nm| nm == name && nm != "_")) {
            return Some((None, named, Vec::new(), Vec::new())); // the REST binder
        }
        return None; // this map does not bind `name`
    }
    // A TUPLE / LIST pattern: try each element position at `Elem(i)`, recursing for a nested map.
    let elems: Option<Vec<StructId>> = if is_tuple_pattern(db, pattern) {
        db.ast
            .compound_form_of(pattern, CompoundCtor::Tuple)
            .map(<[StructId]>::to_vec)
    } else if is_list_pattern(db, pattern) {
        // Only LEADING (fixed) elements compose an `Elem(i)`; a `.. rest` sublist is not descended for a
        // nested map (a runtime sublist read — out of scope, as `find_binder_in_list`'s rest is).
        db.ast
            .compound_form_of(pattern, CompoundCtor::List)
            .map(|t| match db.ast.rest_marker(t).map(|(i, _, _)| i) {
                Some(k) => t[..k].to_vec(),
                None => t.to_vec(),
            })
    } else {
        None
    };
    if let Some(elems) = elems {
        for (i, &elem) in elems.iter().enumerate() {
            let len = path.len();
            path.push(crate::core::PathStep::Elem(i));
            if let Some(hit) = find_map_binder_in_pattern(db, elem, name, path) {
                return Some(hit);
            }
            path.truncate(len);
        }
        return None;
    }
    // A VARIANT pattern `(head arg…)` (head a `(. Sum V)` / bare variant name, not a compound ctor): the
    // payload is reached by a `Payload` step, then each arg (multi-payload → tuple `Elem(i)`) descends.
    let Struct::List(app) = db.ast.get(pattern) else {
        return None;
    };
    if app.len() < 2 {
        return None;
    }
    let head = app[0];
    let is_compound_ctor = db
        .ast
        .as_name(head)
        .is_some_and(|h| matches!(h, "list" | "tuple" | "record" | "map"));
    let head_ok = !is_compound_ctor
        && (db.ast.as_form(head, ".").is_some() || db.ast.as_name(head).is_some());
    if !head_ok {
        return None;
    }
    let len = path.len();
    path.push(crate::core::PathStep::Payload);
    if app.len() == 2 {
        if let Some(hit) = find_map_binder_in_pattern(db, app[1], name, path) {
            return Some(hit);
        }
    } else {
        // Multi-payload `(Cons h t)` = a single tuple payload: each arg descends at `Elem(i)` after Payload.
        for (i, &arg) in app[1..].iter().enumerate() {
            let plen = path.len();
            path.push(crate::core::PathStep::Elem(i));
            if let Some(hit) = find_map_binder_in_pattern(db, arg, name, path) {
                return Some(hit);
            }
            path.truncate(plen);
        }
    }
    path.truncate(len);
    None
}

/// If `form` is a match ARM `(pattern body)` (ascended from its BODY or guard cond) whose pattern is a
/// COMPOUND (tuple/list/variant) with a `(record …)` sub-pattern NESTED inside it binding `name` at a
/// BARE-binder field, return `(scrutinee, path, key)` — the enclosing match's scrutinee, the access `path`
/// from it down to the nested RECORD, and the field's `Symbol`. `None` otherwise. The NESTED companion of
/// [`match_arm_record_binds`] (the DIRECT record scrutinee, Case 6rec); the record twin of
/// [`match_arm_nested_map_binds`]. A non-empty path is required — a TOP-LEVEL record is Case 6rec's job.
/// `heads` carries the variant-constructor head at each `Payload` step in `path` (empty for tuple/list-only
/// nesting), so inference can walk a variant payload down to the nested record.
fn match_arm_nested_record_binds_path(
    db: &Db,
    form: StructId,
    from: StructId,
    name: &str,
) -> Option<(
    StructId,
    Vec<crate::core::PathStep>,
    Symbol,
    Vec<crate::core::PathStep>,
    Vec<StructId>,
)> {
    let Struct::List(pb) = db.ast.get(form) else {
        return None;
    };
    if pb.len() != 2 {
        return None;
    }
    let (pattern, body) = (pb[0], pb[1]);
    // Peel a `(guard <pattern> <cond>)` wrapper — a guard-cond reference binds the same nested binders.
    let (arm_pat, guard_cond) = match db.ast.as_form(pattern, "guard") {
        Some(g) if g.len() == 2 => (g[0], Some(g[1])),
        _ => (pattern, None),
    };
    if from != body && Some(from) != guard_cond {
        return None;
    }
    // `form`'s parent must be a `(match scrutinee arm…)` and `form` an arm (not the scrutinee).
    let parent = db.parent_of(form)?;
    let mtail = db.ast.as_form(parent, "match")?;
    let scrutinee = *mtail.first()?;
    if form == scrutinee {
        return None;
    }
    // A TOP-LEVEL record is Case 6rec's, not here — descend the compound for a nested record sub-pattern.
    let mut path = Vec::new();
    let mut heads = Vec::new();
    let (key, sub_path) = find_record_binder_in_pattern(db, arm_pat, name, &mut path, &mut heads)?;
    if path.is_empty() {
        return None; // the record is the whole pattern → Case 6rec's job
    }
    Some((scrutinee, path, key, sub_path, heads))
}

/// Descend a COMPOUND pattern (tuple/list/variant) looking for a `(record …)` sub-pattern that binds `name`
/// at a BARE-binder field, accumulating the `Elem`/`Payload` access steps to reach that record. On a hit
/// returns `Some(field-key)` with `path` holding the steps from the enclosing pattern's scrutinee to the
/// RECORD (NOT the field — the field is name-keyed via the returned key, resolved at fold). The record
/// analogue of [`find_map_binder_in_pattern`]; only a BARE-binder field is wired (a further compound field
/// value is out of scope — returns `None`, keeping deeper nesting declining via the coded Case
/// 6rec-nested-decline). Element positions compose to any depth. `heads` accumulates the variant-ctor head
/// at each `Payload` step (empty for tuple/list-only nesting), so inference can walk a variant payload down
/// to the nested record — the record-field twin of `SumPayload.heads`.
fn find_record_binder_in_pattern(
    db: &Db,
    pattern: StructId,
    name: &str,
    path: &mut Vec<crate::core::PathStep>,
    heads: &mut Vec<StructId>,
) -> Option<(Symbol, Vec<crate::core::PathStep>)> {
    // A RECORD pattern here: `path` reaches THIS record. Return the field `key` that binds `name`, plus the
    // `sub_path` — the descent BELOW that field's value (empty for a BARE-binder field). A record field is
    // name-keyed (its sorted slot resolves at fold), so no step is pushed onto `path` for the field itself;
    // the `sub_path` is the further `Elem`/`Payload` walk INTO the field value when it is a compound
    // sub-pattern (§235: a field value binder may be any nested pattern).
    if db
        .ast
        .compound_form_of(pattern, CompoundCtor::Record)
        .is_some()
    {
        let fields = db.ast.compound_form_of(pattern, CompoundCtor::Record)?;
        for &pair in fields {
            let Some((key_id, sub)) = record_pattern_field_kv(db, pair) else {
                continue;
            };
            let Some(key) = read_key(db, key_id) else {
                continue;
            };
            if name == "_" {
                continue;
            }
            // BARE-binder field `(= key name)` — `name` IS this field, no descent.
            if db.ast.as_name(sub) == Some(name) {
                return Some((key, Vec::new()));
            }
            // DEEPER: the field VALUE is a compound sub-pattern binding `name` — descend it positionally,
            // collecting the `sub_path` (§235). A TUPLE/LIST field value is positional (resolve-time
            // `Elem`), and a variant descends by `Payload`. A nested RECORD field value would need a
            // deferred name-keyed slot (not a resolve-time step) — NOT yet wired here, so it stays declined
            // (the caller's coded decline names it). A `Payload` step below a field needs a `sub_heads`
            // entry the `RecordField` does not yet carry, so a variant BELOW the field is also deferred:
            // accept ONLY an all-`Elem` sub_path (the tuple/list nesting — the #tuple(c d)-below-field case).
            let (mut sub_path, mut sub_heads) = (Vec::new(), Vec::new());
            let bound = if is_tuple_pattern(db, sub) {
                find_binder_in_tuple(db, sub, name, &mut sub_path, &mut sub_heads)
            } else if is_list_pattern(db, sub) {
                find_binder_in_list(db, sub, name, &mut sub_path, &mut sub_heads)
            } else if db.ast.compound_form_of(sub, CompoundCtor::Record).is_some() {
                false // nested record below a field: deferred name-keyed slot — follow-up
            } else {
                find_binder_in_pattern(db, sub, name, &mut sub_path, &mut sub_heads)
            };
            if bound && sub_heads.is_empty() {
                return Some((key, sub_path));
            }
        }
        return None;
    }
    // A TUPLE / LIST pattern: try each element position at `Elem(i)`, recursing for a nested record. No head
    // is pushed for an `Elem` step (its type comes from tuple/list-indexing, not a variant head).
    let elems: Option<Vec<StructId>> = if is_tuple_pattern(db, pattern) {
        db.ast
            .compound_form_of(pattern, CompoundCtor::Tuple)
            .map(<[StructId]>::to_vec)
    } else if is_list_pattern(db, pattern) {
        // Only LEADING (fixed) elements compose an `Elem(i)`; a `.. rest` sublist is not descended (mirrors
        // `find_map_binder_in_pattern`).
        db.ast
            .compound_form_of(pattern, CompoundCtor::List)
            .map(|t| match db.ast.rest_marker(t).map(|(i, _, _)| i) {
                Some(k) => t[..k].to_vec(),
                None => t.to_vec(),
            })
    } else {
        None
    };
    if let Some(elems) = elems {
        for (i, &elem) in elems.iter().enumerate() {
            let len = path.len();
            path.push(crate::core::PathStep::Elem(i));
            if let Some(hit) = find_record_binder_in_pattern(db, elem, name, path, heads) {
                return Some(hit);
            }
            path.truncate(len);
        }
        return None;
    }
    // A VARIANT pattern `(head arg…)` (head a `(. Sum V)` / bare variant name, not a compound ctor): the
    // payload is reached by a `Payload` step (recording the variant HEAD so inference walks the payload
    // type), then each arg (multi-payload → tuple `Elem(i)`) descends — mirrors `find_map_binder_in_pattern`'s
    // variant arm. Inference's `record_field_at_path` consumes `heads` at each `Payload` (via the same
    // variant-payload walk `SumPayload` uses) to reach the nested `Ty::Record`, then the trailing field
    // read is the name-keyed sorted-slot the lowering appends — so a variant-nested record field grounds to
    // its real type (not `Ty::Any`), the wired twin of the tuple/list nesting.
    let Struct::List(app) = db.ast.get(pattern) else {
        return None;
    };
    if app.len() < 2 {
        return None;
    }
    let head = app[0];
    let is_compound_ctor = db
        .ast
        .as_name(head)
        .is_some_and(|h| matches!(h, "list" | "tuple" | "record" | "map"));
    let head_ok = !is_compound_ctor
        && (db.ast.as_form(head, ".").is_some() || db.ast.as_name(head).is_some());
    if !head_ok {
        return None;
    }
    let (plen, hlen) = (path.len(), heads.len());
    path.push(crate::core::PathStep::Payload);
    heads.push(head);
    if app.len() == 2 {
        if let Some(hit) = find_record_binder_in_pattern(db, app[1], name, path, heads) {
            return Some(hit);
        }
    } else {
        // Multi-payload `(Cons h t)` = a single tuple payload: each arg descends at `Elem(i)` after Payload.
        for (i, &arg) in app[1..].iter().enumerate() {
            let alen = path.len();
            path.push(crate::core::PathStep::Elem(i));
            if let Some(hit) = find_record_binder_in_pattern(db, arg, name, path, heads) {
                return Some(hit);
            }
            path.truncate(alen);
        }
    }
    path.truncate(plen);
    heads.truncate(hlen);
    None
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
            // A LIST destructuring binding `((list x .. rest) V)` — the LHS is a `(list …)` pattern. A
            // reference to a LEADING element binder resolves to a `SumPayload` reading that element (path
            // `[Elem(i), …]`, possibly nested); a reference to the REST binder to `[RestFrom(lead)]` — the
            // tail sublist — EXACTLY as a `(match V ((list x .. rest) …))` arm binder does (resolve Case
            // 6l/6r). Reuses the SAME form-independent walkers those cases use, so the binding position IS a
            // one-arm irrefutable match with zero new IR. Only the REST form is irrefutable (matches any
            // length); a fixed-arity `(list a b)` binding is refutable and rejected at lowering by
            // `check_binding_pattern` (CDZ0210) — this lookup only routes an in-scope binder to its value.
            else if db
                .ast
                .compound_form_of(lhs, crate::ast::CompoundCtor::List)
                .is_some()
            {
                if let Some((path, heads)) = find_leading_binder_in_list_pattern(db, lhs, name) {
                    return Some(Resolved::SumPayload {
                        scrutinee: kv[1],
                        steps: path.into(),
                        heads: heads.into(),
                    });
                }
                if let Some(lead) = find_rest_binder_in_list_pattern(db, lhs, name) {
                    return Some(Resolved::SumPayload {
                        scrutinee: kv[1],
                        steps: vec![crate::core::PathStep::RestFrom(lead)].into(),
                        heads: vec![].into(),
                    });
                }
            }
            // A RECORD destructuring binding `((record (x a) (y b)) V)` — the LHS is a `(record …)` pattern.
            // A record field is projected by NAME (not position), so a field binder `a` in `(record (x a))`
            // resolves to the PROJECTION of the bound value at that field — `(. V x)` — which is EXACTLY
            // `Resolved::Member { operand: V, key: x }`, the same form `(. V x)` resolves to (folding to a
            // `Core::Proj` at the field's SORTED slot when the record type is solved, `lower`'s `Member` arm
            // via `runtime_member_index`). Zero new IR — the binding position IS a one-arm irrefutable match,
            // and a record's field read is the ordinary member access the projection workaround already used.
            // Refutability + linearity + field existence + the bare-binder scope are enforced at lowering by
            // `check_binding_pattern` (Increment B) — this lookup only routes an in-scope field binder to its
            // projection. Only a BARE-binder field value is wired (the scope `check_binding_pattern` admits);
            // a nested compound field value is declined there, so it never reaches a body reference here.
            else if let Some(fields) = db
                .ast
                .compound_form_of(lhs, CompoundCtor::Record)
                .map(<[_]>::to_vec)
            {
                // Split off a trailing `.. rest` — the record open-row REST binder. It binds a RECORD of the
                // UNNAMED fields (the residual), resolving to `Resolved::RecordRest { scrutinee: V, named }`
                // — the SAME form the match-arm Case 6rec-rest produces, just from the binding position (a
                // destructuring param desugars to this `let`, `binding_params::lower`). `named` = the named
                // field key occurrences (removed to form the residual). Its `type_of`/const-fold are
                // origin-agnostic (v-inference), so the residual types + folds unchanged. The named-field
                // Member loop below iterates only the LEADING fields (the rest field is not a `.. `-named
                // field). Irrefutability is enforced at lowering by `check_binding_pattern`.
                let (lead_fields, rest_binder): (&[StructId], Option<StructId>) =
                    match db.ast.rest_marker(&fields) {
                        Some((k, operand, trailing_start)) if trailing_start == fields.len() => {
                            (&fields[..k], Some(operand))
                        }
                        _ => (&fields[..], None),
                    };
                if let Some(rest_b) = rest_binder
                    && db.ast.as_name(rest_b) == Some(name)
                    && name != "_"
                {
                    let named: Vec<StructId> = lead_fields
                        .iter()
                        .filter_map(|&p| {
                            let Struct::List(kv2) = db.ast.get(p) else {
                                return None;
                            };
                            if kv2.len() == 3 && db.ast.as_name(kv2[0]) == Some("=") {
                                Some(kv2[1])
                            } else if kv2.len() == 2 {
                                Some(kv2[0])
                            } else {
                                None
                            }
                        })
                        .collect();
                    return Some(Resolved::RecordRest {
                        scrutinee: kv[1],
                        named: named.into(),
                    });
                }
                for pair in lead_fields {
                    // A record-pattern field is the canonical `(= key binder)` triple (path B — same form
                    // as a value-record field): key = child 1, binder = child 2. A legacy `(key binder)`
                    // pair is tolerated (key = child 0, binder = child 1).
                    let Struct::List(kv2) = db.ast.get(*pair) else {
                        continue;
                    };
                    let (key_id, binder_id) =
                        if kv2.len() == 3 && db.ast.as_name(kv2[0]) == Some("=") {
                            (kv2[1], kv2[2])
                        } else if kv2.len() == 2 {
                            (kv2[0], kv2[1])
                        } else {
                            continue;
                        };
                    if db.ast.as_name(binder_id) == Some(name)
                        && name != "_"
                        && let Some(key) = read_key(db, key_id)
                    {
                        return Some(Resolved::Member {
                            operand: kv[1],
                            key,
                        });
                    }
                }
                // Not a bare-binder field. If `name` is bound NESTED inside a field's compound value
                // (`(record (p (tuple a b)))` — `a`/`b` inside the tuple), that is the not-yet-wired
                // Increment-B case `check_binding_pattern` declines: resolve the reference to a CODED
                // decline that NAMES the real cause, so the body reference reports the unimplemented
                // feature rather than a misleading CDZ0101 "unbound name" (the `let` twin of Case 6rec's
                // match-side suppression). A reference to a genuinely-unbound name (bound by NO field)
                // falls through to the ordinary resolution below and gets its real CDZ0101.
                for pair in &fields {
                    let Struct::List(kv2) = db.ast.get(*pair) else {
                        continue;
                    };
                    // Read the field's VALUE sub-pattern from BOTH the canonical 3-element `(= key value)`
                    // FieldPair (the M3-nativized `#record((= x …))` form) AND the legacy 2-element
                    // `(key value)` pair. Handling only `kv2.len() == 2` MISSED the native `=`-form (the same
                    // `=`-migration miss class as the map effects-fold #6790): a deeper binder below a native
                    // nested field then fell through to a MISLEADING CDZ0101 "unbound name" at the body ref
                    // INSTEAD of this clean decline — while `check_binding_pattern` (and the match-arm Case
                    // 6rec-nested-decline) already declined it, so `check` disagreed with the body-ref
                    // resolution. Now both native + legacy forms reach the decline, suppressing the cascade.
                    let value_pat = if kv2.len() == 3 && db.ast.as_name(kv2[0]) == Some("=") {
                        kv2[2]
                    } else if kv2.len() == 2 {
                        kv2[1]
                    } else {
                        continue;
                    };
                    {
                        let (mut path, mut heads) = (Vec::new(), Vec::new());
                        let bound = if is_tuple_pattern(db, value_pat) {
                            find_binder_in_tuple(db, value_pat, name, &mut path, &mut heads)
                        } else if is_list_pattern(db, value_pat) {
                            find_binder_in_list(db, value_pat, name, &mut path, &mut heads)
                        } else if db
                            .ast
                            .compound_form_of(value_pat, crate::ast::CompoundCtor::Record)
                            .is_some()
                        {
                            record_pattern_binds_name(db, value_pat, name)
                        } else {
                            find_binder_in_pattern(db, value_pat, name, &mut path, &mut heads)
                        };
                        if bound {
                            // CODED decline (CDZ0900) — same umbrella the MATCH-arm twin at
                            // `match_arm_record_binds` emits via `Reject::unsupported`. The binding-path
                            // decline formerly used the UNCODED `Reject::decline`, so the identical
                            // feature-gap surfaced as a bare `error:` in a binding position but
                            // `error [CDZ0900]:` in a match arm — a diagnostic-quality asymmetry (operator
                            // seq-286: every user-facing decline carries a code). Now both paths are CDZ0900.
                            return Some(Resolved::Poison(Reject::unsupported(
                                "a nested compound sub-pattern inside a record binding pattern is not \
                                 supported (a record binding binds fields to bare names; \
                                 destructure a nested field with a further `let`)",
                            )));
                        }
                    }
                }
            }
            // A NESTED record inside a TUPLE / LIST / VARIANT destructuring binding — `((tuple a (record
            // (= x b))) V)` binds `b` to field `x` of the record at element 1 of `V`. The top-level cases
            // above (tuple → `SumPayload`, ctor → `SumPayload`, list, record → `Member`/`RecordRest`) do not
            // descend a compound element INTO a nested record, so `b` fell through to a misleading CDZ0101.
            // `find_record_binder_in_pattern` descends the compound (the SAME walker the match-arm Case
            // 6rec-nested uses) to the nested record + returns the field key + the `Elem`/`Payload` access
            // path; resolve `name` to a `RecordField` reading field `key` off the record at that sub-path of
            // the bound value — the binding twin of Case 6rec-nested (§Patterns Compose). A non-empty path
            // is required (a TOP-LEVEL record is the `Member`/`RecordRest` case above); a nested record inside
            // a RECORD FIELD is NOT descended by the walker (declined above, Increment B — consistent with
            // the match path). Placed AFTER the shape cases so a directly-bound name resolves first.
            {
                let (mut path, mut heads) = (Vec::new(), Vec::new());
                if let Some((key, sub_path)) =
                    find_record_binder_in_pattern(db, lhs, name, &mut path, &mut heads)
                    && !path.is_empty()
                {
                    return Some(Resolved::RecordField {
                        scrutinee: kv[1],
                        path: path.into(),
                        key,
                        sub_path: sub_path.into(),
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
pub(crate) fn let_of_bindings_list(db: &Db, form: StructId) -> Option<StructId> {
    let parent = db.parent_of(form)?;
    let tail = db.ast.as_form(parent, "let")?;
    if tail.first().copied() == Some(form) {
        Some(parent)
    } else {
        None
    }
}

/// If `form` is a `(def (NAME param…) body)`'s SIGNATURE LIST (its parent is a `def` and `form` is that
/// def's first tail element), the `def` form; else `None`. The signature analogue of
/// [`let_of_bindings_list`] — confirms `form` is the parameter list a Case-4b reference ascended into, so
/// an earlier type-valued parameter binds in a later parameter's annotation. A `fn` param list is NOT
/// matched (its params are bare/annotated and do not depend on each other today; only a `def` signature
/// carries the type-valued-parameter surface).
fn def_sig_list_of(db: &Db, form: StructId) -> Option<StructId> {
    let parent = db.parent_of(form)?;
    let tail = db.ast.as_form(parent, "def")?;
    if tail.first().copied() == Some(form) {
        Some(parent)
    } else {
        None
    }
}

/// Resolve `name` against the parameters of a def SIGNATURE LIST `sig` that appear STRICTLY BEFORE the
/// parameter `from` we ascended from — the in-order signature scope (Case 4b). `sig = [NAME, p0, p1, …]`;
/// a parameter is a bare name `p` or an annotated binder `(: p T)`. Returns a `Ref` to the matching
/// parameter's NAME occurrence (the same `Resolved` a body reference to a param produces via
/// `binder_in_scope`), last-of-`name`-before-`from` wins. `NAME` at index 0 is the def name, never a
/// parameter, so it is skipped. `None` if no earlier parameter is named `name`.
fn param_binder_before(db: &Db, sig: StructId, name: &str, from: StructId) -> Option<Resolved> {
    let Struct::List(children) = db.ast.get(sig) else {
        return None;
    };
    // The window end: parameters strictly before `from` (a direct child of `sig` — its `child_ix` is O(1)).
    let k = db.child_ix_of(from);
    if children.get(k) != Some(&from) {
        return None; // `from` is not a direct child of this signature (defensive)
    }
    // `from` at index 0 is the def NAME (not a parameter) or index 1 is the first param — either way there
    // is no EARLIER parameter, so the window `[1, k)` is empty. Guard the slice (avoid `1..0` underflow).
    if k <= 1 {
        return None;
    }
    // Scan [1, k) in REVERSE (skip NAME at 0; last-before-`from` wins). Each element is a parameter binder.
    for &p in children[1..k].iter().rev() {
        // A parameter is a bare name `p` or an annotated binder `(: p T)` — take the NAME occurrence from
        // either (a bare name atom directly, or the FIRST operand of a `(: name T)` form).
        let name_occ = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(occ) => occ,
            None => p,
        };
        if db.ast.as_name(name_occ) == Some(name) {
            return Some(Resolved::Ref { value: name_occ });
        }
    }
    None
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
/// mechanical edit). Shared by `if` (want 3), `and`/`or` (want 2), `not` (want 1), `resume`/`host`/`let`/
/// `fn` (want 2), and `compile::collect_faults`' def-body-count check (want 2).
pub(crate) fn fixed_arity_reject(
    id: StructId,
    tail: &[StructId],
    want: usize,
    message: &str,
) -> Reject {
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
        // The COMMON too-few case is a MISSING ELSE — `(if b then)`, the reflex of a language where `if`
        // without `else` is a statement. In Cadenza an `if` is an EXPRESSION (it must produce a value on
        // both branches), so the actionable repair is to ADD an else branch. Offer an `InsertArms` fix
        // appending an `(trap "TODO")` else — a diverging placeholder (`trap : ∀a. String → a`) that
        // inhabits ANY type, so it type-checks against the then-branch whatever its type, clearing the
        // arity error in one shot for the author to fill (the `if` twin of the non-exhaustive-match
        // add-arm fix). TOO MANY operands keeps the surplus-delete fix from `fixed_arity_reject`.
        if tail.len() == 2 {
            return Resolved::Poison(
                Reject::coded(
                    Code::Malformed,
                    "this `if` has no else branch — an `if` is an expression, so it needs both a then \
                     and an else branch: `(if <cond> <then> <else>)`",
                )
                .at(id)
                .with_fix(crate::diag::Fix::insert_arms_heuristic(
                    id,
                    vec!["(trap \"TODO\")".to_string()],
                )),
            );
        }
        // ML-SYNTAX-IN-THE-S-EXPR-SURFACE. A user who knows the ML surface writes `(if b then 1 else 0)`,
        // reaching for the `then`/`else` KEYWORDS — but this s-expr surface's `if` is three POSITIONAL forms
        // `(if <cond> <then> <else>)` with no keywords. The stray `then`/`else` land as bare operands, so the
        // form reads as 5 operands and hits the generic "if takes exactly 3 operands" arity reject — which
        // names the arity, not the real mistake (used ML if-syntax). Detect the signature (5 operands with
        // the 2nd and 4th being the bare symbols `then` and `else`) and name the actual confusion + the
        // correct shape, so the author fixes the syntax rather than puzzling over "3 operands". Flagged by
        // v-compiler-ml (a trap that cost them several ticks root-causing). Keyed on the exact `then`/`else`
        // atom positions, so an ordinary too-many-operand `if` keeps its generic arity reject + delete fix.
        if tail.len() == 5
            && db.ast.as_name(tail[1]) == Some("then")
            && db.ast.as_name(tail[3]) == Some("else")
        {
            return Resolved::Poison(
                Reject::coded(
                    Code::Malformed,
                    "this `if` uses the ML `then`/`else` keywords, but the s-expr surface's `if` is three \
                     positional forms with no keywords: `(if <cond> <then> <else>)` — write `(if <cond> \
                     <then-value> <else-value>)`, dropping `then` and `else`",
                )
                .at(id),
            );
        }
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

/// Resolve `(try e)` — the fallible short-circuit operator (`DESIGN-try-operator-rcdzc.md`), the s-expr
/// form of the ML postfix `e?`. EXACTLY ONE operand (the `Result`/`Option` expression it unwraps). A
/// zero- or multi-operand `(try …)` is malformed with the surplus-delete fix (`arity_one_operand_reject`,
/// shared with `quote`), mirroring `resolve_not`'s fixed-arity check. The well-formed node is carried
/// first-class through infer (so a type error / a missing-boundary CDZ0230 points at the `?`) and
/// desugared at Hir→Mir against the enclosing boundary.
fn resolve_try(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "try").unwrap_or(&[]);
    if tail.len() != 1 {
        return Resolved::Poison(arity_one_operand_reject(
            id,
            tail,
            "try takes exactly one operand — the fallible expression it unwraps",
        ));
    }
    Resolved::Try { operand: tail[0] }
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
        "quote produces an AST value, which is not supported",
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
    // An `unquote`/`unquote-splicing` is meaningful ONLY inside a `` ` `` template — a `,`/`,@` with no
    // enclosing quasiquote has no template to insert into, so it is a syntax error (CDZ0003).
    //= spec/capabilities/metaprogramming.md#quasiquote-constructs-ast-with-selective-evaluation
    //# Unquote and unquote-splicing outside a quasiquote context MUST be a syntax error.
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
        "an unquote inside a quasiquote builds an AST value, which is not supported",
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
        "quasiquote produces an AST value, which is not supported",
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
        // A UTF-8 string segment `(utf8 s n)` — read `n` bytes and decode them as strict UTF-8, binding
        // `s : String` on success, a non-match on ill-formed input. The size is REQUIRED (always
        // dependent): a String segment with no length has no boundary, so there is no unsized form.
        if kind_name == "utf8" {
            if parts.len() != 3 {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "a utf8 bin segment is (utf8 s n)",
                ));
            }
            segs.push(crate::resolved::Segment {
                kind: crate::resolved::SegKind::Utf8 { size: parts[2] },
                slot: parts[1],
                little_endian: false,
            });
            continue;
        }
        // A `uNN`/`iNN` head whose NN is not one of the aliased byte widths {8, 16, 32, 64} — `u24`, `u7`,
        // `u128`. It IS the `uNN`/`iNN` SHAPE the generic message points at, so blaming an "unrecognized
        // kind" misleads (the author wrote what the hint says). Name the real limit — the supported widths
        // — and point a non-byte-aligned width at the `(bits v k)` segment, which takes an arbitrary width.
        if let Some(rest) = kind_name
            .strip_prefix('u')
            .or_else(|| kind_name.strip_prefix('i'))
            && !rest.is_empty()
            && rest.bytes().all(|b| b.is_ascii_digit())
        {
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a fixed-width integer bin segment must be one of u8/u16/u32/u64 or i8/i16/i32/i64 \
                     (the byte-aligned widths) — `{kind_name}` is not; for an arbitrary bit width use a \
                     `(bits v k)` segment"
                ),
            );
            // A `uNN`/`iNN` whose width is a near-miss of a real byte-aligned kind (`u166`→`u16`,
            // `u62`→`u64`, `u17`→`u16`) carries a rename fix on the kind head — the applyable half the
            // "u62→u64 handled above" comment promised. HEURISTIC: it is a guess at a mistyped width, and
            // the message still names `(bits v k)` for an author who genuinely wanted a non-aligned width,
            // so both routes stay visible. The candidate pool is ONLY the SAME-signedness byte-aligned
            // widths (a `u166` never suggests an `i16` — signedness is not the slip), so the suggestion
            // stays a plausible width fix. A width too far from any byte-aligned kind (`u128`, `u9999`) is
            // beyond `nearest`'s cutoff → no fix, just the `(bits v k)` guidance.
            let signed = kind_name.starts_with('i');
            const U_KINDS: &[&str] = &["u8", "u16", "u32", "u64"];
            const I_KINDS: &[&str] = &["i8", "i16", "i32", "i64"];
            let pool = if signed { I_KINDS } else { U_KINDS };
            if let Some(suggestion) = crate::diag::suggest::nearest(kind_name, pool.iter().copied())
            {
                let head = parts[0];
                reject = reject
                    .at(head)
                    .with_fix(crate::diag::Fix::replace_heuristic(head, suggestion));
            }
            return Resolved::Poison(reject);
        }
        // An unrecognized kind head. If it is a plausible typo of a known kind — `byte`→`bytes`,
        // `utf`/`utf-8`→`utf8`, `bit`→`bits`, `u62`→`u64` handled above — name it and carry a rename fix on
        // the kind-head node (the bin-segment twin of the member/variant did-you-mean). The candidate set is
        // the CLOSED bin vocabulary. `parts[0]` is the head to rewrite.
        let mut reject = Reject::coded(
            Code::Malformed,
            "an unrecognized bin segment kind (expected uNN/iNN/bits/bytes/utf8)",
        )
        .at(seg);
        const BIN_KINDS: &[&str] = &[
            "bits", "bytes", "utf8", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64",
        ];
        if let Some(suggestion) =
            crate::diag::suggest::nearest(kind_name, BIN_KINDS.iter().copied())
        {
            let head = parts[0];
            reject = Reject::coded(
                Code::Malformed,
                format!(
                    "an unrecognized bin segment kind `{kind_name}` — did you mean `{suggestion}`? \
                     (expected uNN/iNN/bits/bytes/utf8)"
                ),
            )
            .at(head)
            .with_fix(crate::diag::Fix::replace_heuristic(head, suggestion));
        }
        return Resolved::Poison(reject);
    }
    Resolved::Bin { segs: segs.into() }
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
        args: std::rc::Rc::from([lhs]),
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
///
/// The handler that discharges a performed operation is thus determined from the RESOLVED IR: a `handle`
/// is a resolved node and a perform inside its body is matched to the enclosing handler's arms at resolve
/// /lower time — so the discharging handler is fixed BEFORE instruction selection, not accumulated as
/// state while the backend emits instructions.
//= spec/capabilities/compiler-pipeline.md#the-compiler-resolves-names-before-it-selects-instructions
//# The compiler MUST determine the handler that discharges each performed effect operation from the structure of the resolved intermediate representation, so that the discharging handler of an operation is fixed before instruction selection rather than by state accumulated while instructions are emitted.
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
                "a handle arm must be `(<op> (params…) <state> <body>)` \
                 or `(<op> (params…) <state> <k> <body>)` (the `ctl`-style continuation form)",
            ));
        };
        // FOUR parts = the tail/abortive form `(op (params) state body)`; FIVE parts = the general
        // `ctl`-style form `(op (params) state k body)`, where `k` binds the delimited CONTINUATION as a
        // first-class value (E5). The last part is always the body; a 5th (middle) part is the `k` binder.
        let (cont, state, body) = match parts.len() {
            4 => (None, parts[2], parts[3]),
            5 => (Some(parts[3]), parts[2], parts[4]),
            _ => {
                return Resolved::Poison(Reject::coded(
                    Code::Malformed,
                    "a handle arm must be `(<op> (params…) <state> <body>)` \
                     or `(<op> (params…) <state> <k> <body>)` (the `ctl`-style continuation form)",
                ));
            }
        };
        let op = parts[0];
        let params: std::rc::Rc<[StructId]> = match db.ast.get(parts[1]) {
            Struct::List(ps) => ps.as_slice().into(),
            // A single bare param (no parens) — treat as one param. `()` is the empty list (nullary).
            _ => std::rc::Rc::from([parts[1]]),
        };
        arms.push(HandleArm {
            op,
            params,
            state,
            cont,
            body,
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
    // TOO MANY operands — `(resume v s extra)`. A resume is exactly `(resume <value> <next-state>)`; a
    // surplus operand was SILENTLY IGNORED (the resolver read only `tail[0]`/`tail[1]`), compiling to code
    // that dropped the extra — a silent miscompile. Reject it CDZ0201 with a delete-the-surplus fix (the
    // same fixed-arity surplus-delete `if`/`and`/`not` get via `fixed_arity_reject`), so a too-many resume
    // reports the arity defect + how to fix it rather than compiling wrong.
    if tail.len() > 2 {
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            2,
            &format!("this resume has too many operands — {SHAPE}"),
        ));
    }
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
            // The effect list is present but the trailing BODY is missing. Append an `(trap "TODO")` body
            // placeholder — the `host` twin of the `let`/`fn`-no-body add-fix (`trap` inhabits any type).
            return Resolved::Poison(
                Reject::coded(Code::Malformed, format!("this host has no body — {SHAPE}"))
                    .at(id)
                    .with_fix(crate::diag::Fix::insert_arms_heuristic(
                        id,
                        vec!["(trap \"TODO\")".to_string()],
                    )),
            );
        }
    };
    let effects: std::rc::Rc<[StructId]> = match db.ast.get(effects_occ) {
        Struct::List(es) => es.as_slice().into(),
        _ => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this host's effects must be a list — {SHAPE}"),
            ));
        }
    };
    // TOO MANY operands — `(host (E) body extra)`. A host is exactly `(host (<effect>…) <body>)`; a
    // surplus operand after the body was SILENTLY IGNORED (the resolver read only `tail[0]`/`tail[1]`),
    // compiling to code that dropped the extra — a silent miscompile, the `host` analogue of the too-many
    // `resume` gap. Reject it CDZ0201 with a delete-the-surplus fix (the shared `fixed_arity_reject`), so
    // a too-many host reports the arity defect + how to fix it rather than compiling wrong.
    if tail.len() > 2 {
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            2,
            &format!("this host has too many operands — {SHAPE}"),
        ));
    }
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
            // The bindings are present but the trailing BODY is missing — `(let ((x 5)))`. The actionable
            // repair is to ADD a body; append an `(trap "TODO")` placeholder (`trap : ∀a. String → a`
            // inhabits any type, so it type-checks in the body position whatever the `let` is used at) — the
            // `let` twin of the missing-`if`-else add-fix. `cdz fix` fills the `(let (…) <here>)` slot.
            return Resolved::Poison(
                Reject::coded(Code::Malformed, format!("this let has no body — {SHAPE}"))
                    .at(id)
                    .with_fix(crate::diag::Fix::insert_arms_heuristic(
                        id,
                        vec!["(trap \"TODO\")".to_string()],
                    )),
            );
        }
    };
    let pairs = binding_pairs(db, bindings_occ);
    if pairs.is_empty() {
        // Distinguish an EMPTY binding list `(let () <body>)` from a MALFORMED one `(let ((a 1 2)) …)`.
        // Both leave `pairs` empty, but the former has no binding to be "malformed" — its real issue is
        // that it binds nothing, so the `let` is pointless (the body can be written directly). Name THAT
        // instead of the misleading "each must be `(<name> <init>)`" (which implies a broken binding that
        // isn't there). A `bindings_occ` that is an empty List is the `()` case; anything else (a non-list,
        // or a list whose children are all non-`(<name> <init>)`) is genuinely malformed.
        let is_empty_list =
            matches!(db.ast.get(bindings_occ), Struct::List(items) if items.is_empty());
        let message = if is_empty_list {
            format!(
                "this let binds nothing — an empty `()` binding list has no effect; write the body \
                 directly, or add a `(<name> <init>)` binding. {SHAPE}"
            )
        } else {
            format!("this let's bindings are malformed — each must be `(<name> <init>)`. {SHAPE}")
        };
        return Resolved::Poison(Reject::coded(Code::Malformed, message));
    }
    // TOO MANY operands — `(let (binds) body extra)`. A let is exactly `(let ((<name> <init>)…) <body>)`
    // with ONE body; a surplus operand after the body was SILENTLY IGNORED (the resolver read only
    // `tail[0]`/`tail[1]`), compiling to code that dropped it — a silent miscompile, and a likely author
    // slip (expecting `do`-style sequencing where a let takes a single body). Reject it CDZ0201 with a
    // delete-the-surplus fix (the shared `fixed_arity_reject`); the message names `do` as the sequencing
    // form so the author knows how to write multiple statements.
    if tail.len() > 2 {
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            2,
            &format!(
                "this let has more than one body — a let takes a single body (wrap multiple \
                 statements in a `(do …)`). {SHAPE}"
            ),
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

pub(crate) fn read_key(db: &Db, node: StructId) -> Option<Symbol> {
    if let Some(n) = db.ast.as_name(node) {
        return Some(Symbol::plain(n));
    }
    // A `#symbol` LITERAL (`#field` / `#"field"`, a `Leaf::Sym`) read as a plain field LABEL — the
    // 3-operand row-op field selector (`Record.with r #field v`). The name is taken at RESOLVE time as a
    // static label (NOT demanded as a `Ty::Symbol` value that flows through inference), exactly like the
    // bare-name and `(meta …)` cases: `#field` is where the label comes FROM, not a change to whether it
    // is static (DESIGN-record-update-syntax.md — the row-op field name stays a compile-time label).
    if let Some(s) = db.ast.as_sym(node) {
        return Some(Symbol::plain(s));
    }
    // `(meta NAME)` → the symbol `NAME` in the `meta` namespace.
    if let Some(tail) = db.ast.as_form(node, "meta")
        && let Some(name) = tail.first().and_then(|&s| db.ast.as_name(s))
    {
        return Some(Symbol {
            namespace: Some("meta".into()),
            name: name.into(),
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
            // `Rational` — the exact-rational leaf, paired with `encode_ty`'s `Rational` arm so a
            // `Rational` nested in a compound type-value round-trips faithfully (not collapsing to `Unit`).
            "Rational" => Some(Ty::Rational),
            "Float32" => Some(Ty::Float(crate::ty::FloatTy::fixed(32))),
            "Float64" => Some(Ty::Float(crate::ty::FloatTy::fixed(64))),
            // The KIND-OF-TYPES `Ty::Type`, the dual of `eval::encode_ty`'s `Type` arm. Reaches here when a
            // `Ty::Type` is encoded inside a compound type-value that round-trips (e.g. `(-> Type Int64)` →
            // `Ty::Fn(Ty::Type, Int64)`); without it the bare name decoded to `None` and the enclosing type
            // collapsed the `Type` position to `Unit`. This decodes a bare name INSIDE a compiler-emitted
            // `(typeval …)` node (not a user type annotation — the reflection module is recognized
            // structurally elsewhere), so it is internal + consistent, not a prelude name-key.
            "Type" => Some(Ty::Type),
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
        // A TYPE VARIABLE — `(Var N)`, the dual of `eval::encode_ty`'s `Var` arm. Reaches here when a
        // type-valued parameter is encoded inside a compound type-value that round-trips (e.g. the arrow
        // `(-> t Int64)` → `Ty::Fn(Var(t), Int64)`); without this arm the `Var` decoded to `None` and the
        // whole enclosing type failed to decode, collapsing the parameter to `Unit`.
        "Var" => {
            let tail = db.ast.as_form(node, "Var")?;
            let n = match db.ast.get(*tail.first()?) {
                Struct::Atom(l) => match db.ast.leaf(*l) {
                    Leaf::Int { value, .. } => {
                        value.to_i64().and_then(|n| u32::try_from(n).ok())?
                    }
                    _ => return None,
                },
                _ => return None,
            };
            Some(Ty::Var(n))
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
            // The NAME token is read (to validate the wire shape) but DISCARDED — the name is no longer
            // carried on `Ty::Sum` (identity is `decl + args`; render recovers it from `decl`).
            let _ = db.ast.as_name(*tail.first()?)?;
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
            Some(db.normalize_sum(decl, args))
        }
        // A nominal type-value: `(Nominal <name> <decl> (args…) <inner>)` — the dual of `eval::encode_ty`'s
        // `Nominal` arm. Carries its own `decl + args` (identity) and encoded `inner` (machine-rep hint),
        // so it round-trips independently of `newtype_inner` (an already-built `Ty::Nominal` re-encoded,
        // e.g. through `reduce_ctor`).
        "Nominal" => {
            let tail = db.ast.as_form(node, "Nominal")?;
            // NAME read for wire-shape validation but DISCARDED (recovered from `decl` at render time).
            let _ = db.ast.as_name(*tail.first()?)?;
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
                args: args.into(),
                inner: std::rc::Rc::new(inner),
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
            // The unit is a list of `(base NAME EXP)` triples PLUS an optional trailing `(scale NUM DEN)`
            // item (present only for a NON-reference unit — see `encode_ty`'s Qty arm). Restore the scale
            // so a `kilometer`/`foot`/`KiB` param annotation keeps its factor across the round-trip instead
            // of collapsing to its reference unit (which caused a raw-magnitude cross-scale miscompile).
            let mut scale: Option<(i128, i128)> = None;
            for &pair in unit_items {
                if let Some(scale_items) = db.ast.as_form(pair, "scale") {
                    if scale_items.len() != 2 {
                        return None;
                    }
                    let read = |db: &Db, n: StructId| -> Option<i128> {
                        match db.ast.get(n) {
                            Struct::Atom(l) => match db.ast.leaf(*l) {
                                Leaf::Int { value, .. } => value.to_i128(),
                                _ => None,
                            },
                            _ => None,
                        }
                    };
                    scale = Some((read(db, scale_items[0])?, read(db, scale_items[1])?));
                    continue;
                }
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
            if let Some((sn, sd)) = scale {
                unit = unit.scaled(sn, sd)?;
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
            for &field in tail {
                // A field is EITHER the canonical `(: name T)` ascription (the shared binder node —
                // DESIGN-record-type-syntax Phase A) OR the legacy `(name T)` head-app pair. Read both;
                // accepting the ascription is the additive half of RT3, the dual of the `encode_ty`
                // flip and needed before it. Strictly widening — an ascription previously failed the
                // `len == 2` pair check and returned `None`. Pair arm pruned once head-app is extinct.
                let (name_occ, ty_occ) = if let Some(asc) = db.ast.as_form(field, ":") {
                    match asc {
                        [name, t] => (*name, *t),
                        _ => return None,
                    }
                } else {
                    match db.ast.get(field) {
                        Struct::List(items) if items.len() == 2 => (items[0], items[1]),
                        _ => return None,
                    }
                };
                let name = db.ast.as_name(name_occ)?.to_string();
                let t = decode_ty(db, ty_occ)?;
                fields.insert(crate::resolved::Symbol::plain(name), t);
            }
            Some(Ty::Record(std::rc::Rc::new(fields)))
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
    // A HANDLE-ARM CONTINUATION binder `k` — the 4th element of a FIVE-part `ctl`-style arm
    // `(op (params) state k body)` (a 4-part arm has no such slot; its 4th element is the body). Like the
    // state binder it sits directly as an arm element, so `parent` is the arm. Without this, a `k`
    // reference that reaches resolution through `is_param_occurrence` (e.g. a `k` inside a MATCH within the
    // arm body) is not recognized as bound → a spurious CDZ0101 unbound `k` (`s`/params worked, `k` did
    // not — the missing case). `handle_arm_binds` already resolves the reference; this recognizes the
    // binder OCCURRENCE so its scope is established on every path.
    if is_handle_arm(db, parent)
        && let Struct::List(parts) = db.ast.get(parent)
        && parts.len() == 5
        && parts.get(3).copied() == Some(id)
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
            // The parameter list is present but the trailing BODY is missing — `(fn (x))`. Append an
            // `(trap "TODO")` body placeholder (the `fn` twin of the `let`-no-body / missing-`if`-else
            // add-fix): `trap : ∀a. String → a` inhabits any result type, so the completed `(fn (x) (trap
            // "TODO"))` type-checks wherever the lambda is used. `cdz fix` fills the `(fn (…) <here>)` slot.
            return Resolved::Poison(
                Reject::coded(Code::Malformed, format!("this fn has no body — {SHAPE}"))
                    .at(id)
                    .with_fix(crate::diag::Fix::insert_arms_heuristic(
                        id,
                        vec!["(trap \"TODO\")".to_string()],
                    )),
            );
        }
    };
    // The parameter occurrences (each a bare name). Collected into the `Rc<[StructId]>` the variant
    // holds (a refcounted slice — cloning the lambda is then O(1)).
    let params: std::rc::Rc<[StructId]> = match db.ast.get(params_occ) {
        Struct::List(ps) => ps.clone().into(),
        _ => {
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                format!("this fn's parameters must be a list — {SHAPE}"),
            ));
        }
    };
    // TOO MANY operands — `(fn (params) body extra)`. A fn is exactly `(fn (<param>…) <body>)` with ONE
    // body; a surplus operand after the body was SILENTLY IGNORED (the resolver read only
    // `tail[0]`/`tail[1]`), compiling to code that dropped it — a silent miscompile, and a likely author
    // slip (expecting `do`-style sequencing where a fn body is a single expression). Reject it CDZ0201
    // with a delete-the-surplus fix (the shared `fixed_arity_reject`); the message names `do` as the
    // sequencing form, matching the `let` too-many-body reject.
    if tail.len() > 2 {
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            2,
            &format!(
                "this fn has more than one body — a fn takes a single body (wrap multiple \
                 statements in a `(do …)`). {SHAPE}"
            ),
        ));
    }
    Resolved::Lambda { params, body }
}

/// Resolve `(record (k1 v1) (k2 v2) …)`. Field keys are labels (symbols, possibly `(meta …)`-
/// namespaced), NOT resolved. A duplicate field name makes the field set ill-defined → CDZ0201
/// (`core-semantics.md` §A Record Has A Fixed Set Of Named Fields); the check is over the WHOLE field
/// list, not adjacent pairs.
fn resolve_record(db: &Db, id: StructId) -> Resolved {
    let tail = db
        .ast
        .compound_form_of(id, CompoundCtor::Record)
        .unwrap_or(&[]);
    match read_record_fields(db, tail) {
        Ok(fields) => Resolved::Record {
            fields: std::rc::Rc::new(fields),
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
        // A value-record field is the canonical `(= name value)` ascription triple (DESIGN-record-type-
        // syntax Phase B): key = child 1, value = child 2, `=` head dropped. The legacy `(name value)`
        // pair is still accepted (an un-migrated node / hand-built AST), so both shapes read.
        // The canonical `(= name value)` ascription triple (Phase B): read via the shared
        // `Arenas::field_pair` (the ONE `(= k v)` reader for record fields + map entries).
        let (key_id, val_id) = if let Some(kv) = db.ast.field_pair_parts(field) {
            // Native FIELD_PAIR-leaf-headed entry (M2, what the reader now emits for #record fields).
            kv
        } else if let Some(kv) = db.ast.field_pair(field) {
            kv
        } else {
            match db.ast.get(field) {
                // A field LED BY `=` but not exactly 3 elements is a MALFORMED ascription field — e.g.
                // `(= a)` (a field named `a` with NO value, the migrated form of the ill-formed `(a)`), or
                // `(= a 1 2)` (surplus). It is NOT a legacy pair (that would misread the `=` head as the key).
                // Fixed-arity reject anchored at the entry (want the 3-element `(= key value)`).
                Struct::List(kv)
                    if db.ast.as_name(kv.first().copied().unwrap_or(field)) == Some("=") =>
                {
                    return Err(fixed_arity_reject(
                        field,
                        kv,
                        3,
                        "record field must be (= key value)",
                    ));
                }
                // seq-276: a bare `(name value)` VALUE-record entry is NO LONGER accepted — require the
                // canonical FieldPair `(= name value)` (operator: "prefer `=` for records/maps"). Head is
                // NOT `=` here (the `=` spellings were caught above), so a 2-element list is a bare pair.
                // VALUE-scoped only: this arm lives inside `read_record_fields` (value records), so TYPE
                // fields (via `typeval_of`) and record/map PATTERN readers (`map_pattern_of`) are untouched.
                Struct::List(kv) if kv.len() == 2 => {
                    return Err(Reject::coded(
                        Code::Malformed,
                        "record field must be (= key value) — a bare `(key value)` entry is no longer \
                         accepted (add the leading `=`)",
                    )
                    .at(field));
                }
                Struct::List(kv) => {
                    // A wrong-arity non-`=` field entry `(x 1 2)` / `(x)`. A SURPLUS element gets the shared
                    // delete-the-surplus fix; too few is message-only. Anchored at the offending entry.
                    return Err(fixed_arity_reject(
                        field,
                        kv,
                        2,
                        "record field must be (= key value)",
                    ));
                }
                _ => {
                    return Err(Reject::coded(
                        Code::Malformed,
                        "record field must be (= key value)",
                    ));
                }
            }
        };
        let label = match read_key(db, key_id) {
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
        if fields.insert(label.clone(), val_id).is_some() {
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
/// Build the CDZ0214 reject for `(. ty key)` when `ty`'s HANDLE is visible in `id`'s file but the
/// specific constructor `key` is NOT — an abstract import (no constructors) or a partially-concrete
/// import that did not name this constructor. Returns `None` (no reject; leave the member access alone)
/// when: not a linked package; `ty` is not a file-scoped type handle here; `key` is not a genuine variant
/// of `ty` (a later-phase method like `T.expect`); or the constructor IS visible in this file (own type,
/// wildcard `T.*`, or a `(. T key)` that named it). The message names the type + constructor, states the
/// handle-exported-but-this-constructor-withheld reason, and steers to the module's exported functions —
/// the machine-actionable "use the door" fix (Amendment 0.5.0 / `diagnostics.md`).
//= spec/capabilities/modules-and-namespaces.md#a-type-s-handle-and-its-constructors-are-independently-visible
//# A module that makes a type's handle visible without making a constructor visible MUST render that constructor unreachable outside the module — a construction or a match through that constructor in another module MUST be a compile-time rejection carrying the machine-readable code for a withheld constructor — so that a value of such a type is built and deconstructed outside the module only through the functions the module exports, and an invariant the module's constructor establishes cannot be bypassed by another module fabricating a value directly.
fn withheld_ctor_reject(db: &Db, id: StructId, ty: &str, key: &Symbol) -> Option<Reject> {
    if !db.is_linked_package() {
        return None;
    }
    // `ty` must be a type handle VISIBLE in this file (an imported or own type). Its synth record → decl
    // (via `type_decl_by_synth`, so a renamed alias still finds the right variant set).
    let synth = db.file_scoped_type(id, ty).and_then(Result::ok)?;
    let decl = db.type_decl_by_synth(synth)?;
    // Only a genuine variant is a withheld CONSTRUCTOR; a non-variant member declines as a later phase.
    if !decl.variants.iter().any(|v| *v.name == *key.name) {
        return None;
    }
    // If this constructor IS visible in the file, the access is legitimate — no reject. This is a
    // QUALIFIED `(. T A)` access, so consult the qualified surface: a prelude-named ctor (`Ast.Int`)
    // is omitted from the BARE map (so bare `Int` stays the width type) but is fully reachable through
    // the qualified path, and must NOT be reported as withheld here.
    if matches!(
        db.file_scoped_variant_ctor_qualified(id, &key.name),
        Some(Ok(_))
    ) {
        return None;
    }
    Some(
        Reject::coded(
            Code::AbstractCtor,
            format!(
                "`{ty}`'s constructor `{ctor}` is not exported to this file: `{ty}`'s handle is visible \
                 but `{ctor}` is withheld, so a value of `{ty}` cannot be constructed or matched through \
                 `{ctor}` here — obtain and inspect one through the functions the module that declares \
                 `{ty}` exports (or export `{ty}.*` to make every constructor public)",
                ctor = key.name
            ),
        )
        .at(id),
    )
}

fn resolve_member(db: &Db, id: StructId) -> Resolved {
    // Native MEMBER-leaf head (M2, what the reader now emits for `.`/`obj.key`) OR the legacy `Name(".")`
    // head. Read `(operand key)` from whichever; the rest works on the 2-element tail unchanged.
    let member_tail: Vec<StructId>;
    let tail: &[StructId] = if let Some((operand, key)) = db.ast.member_parts(id) {
        member_tail = vec![operand, key];
        &member_tail
    } else {
        db.ast.as_form(id, ".").unwrap_or(&[])
    };
    if tail.len() != 2 {
        // `(. operand key)` is a fixed-arity form (want 2), so route it through the SHARED
        // `fixed_arity_reject` the other fixed-arity forms (`if`/`and`/`not`/`resume`/`let`/`fn`) use — a
        // TOO-MANY access (`(. r x y)`, an over-chained member) gets the delete-the-surplus fix (`cdz fix`
        // removes extras until `(. r x)` remains — for a deeper chain the author writes `(. (. r x) y)`),
        // and a TOO-FEW access (`(. r)`, no key) keeps a message-only reject (supplying the key is not a
        // mechanical edit). Before this, member access was the one fixed-arity form with a terse fix-less
        // message — the fix-parity-across-the-family gap.
        return Resolved::Poison(fixed_arity_reject(
            id,
            tail,
            2,
            "member access is `(. operand key)` — an operand and a single key (a field name or a tuple \
             index); for a nested access chain them: `(. (. r a) b)`",
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
            return Resolved::Poison(Reject::coded(
                Code::Malformed,
                "a computed member key is not supported; use a Map for dynamic keyed access",
            ));
        }
    };
    // WHOLE-MODULE ALIAS PROJECTION: `(. alias member)` where `alias` was bound by `(import "path" alias)`
    // resolves to the aliased module's EXPORTED def `member` — the collision-free path for a uniformly-
    // named export (`descriptor`) imported from 2+ modules (v-platform-itest's multi-contract dispatch).
    // Resolved against the ALIASED file's own export surface (`export_def_in_file`), file-scoped so it
    // reaches exactly that module's def (not a same-named def elsewhere). Only DEFS project today; a type/
    // constructor via `(. alias T)`/`(. alias T.C)` is a separate, larger gap (module records do not
    // project types/ctors at all yet), so a non-def member falls through to the ordinary `Member` path.
    if let Some(alias) = db.ast.as_name(operand)
        && let Some(from_file) = db.module_alias_target(id, alias)
        && let Some(d) = db.export_def_in_file(from_file, &key.name)
    {
        trace!(target: "rcdzc::resolve", node = id.0, alias, member = %key.name, from_file, "member → module-alias export projection");
        return def_as_resolved(db, d, &key.name);
    }
    // WITHHELD-CONSTRUCTOR ACCESS: `(. T A)` where `T`'s handle is visible in this file but constructor
    // `A` is NOT (an abstract import — no ctors — or a partially-concrete import that did not name `A`)
    // is a CDZ0214. The constructor is hidden on purpose, so it is unreachable even through the qualified
    // member path (the bare path is already blocked by file-scoped ctor resolution). Distinguished from a
    // plain unbound name: the type IS visible, this constructor is not. Only fires for a genuine variant
    // of `T`; a non-variant member (a later-phase method like `T.expect`) declines as before.
    if let Some(ty) = db.ast.as_name(operand)
        && let Some(reject) = withheld_ctor_reject(db, id, ty, &key)
    {
        return Resolved::Poison(reject);
    }
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
    let elems: std::rc::Rc<[StructId]> = db
        .ast
        .compound_form_of(id, CompoundCtor::Tuple)
        .unwrap_or(&[])
        .into();
    Resolved::Tuple { elems }
}

/// Resolve `(list e0 e1 …)` — a homogeneous sequence literal. Every element is an AST occurrence in
/// order (resolved on demand); unlike a tuple the elements are NOT per-position (they all unify to one
/// element type — `infer`/`type_errors` enforce homogeneity). An empty `(list)` has no elements — a
/// list of a deferred element type.
fn resolve_list(db: &Db, id: StructId) -> Resolved {
    let elems: std::rc::Rc<[StructId]> = db
        .ast
        .compound_form_of(id, CompoundCtor::List)
        .unwrap_or(&[])
        .into();
    Resolved::List { elems }
}

/// Resolve `("set" e0 e1 …)` — a first-class tagged set construction (operator ruling: pulled all the way
/// through the compiler). Like a list, the elements are AST occurrences in order that all unify to one
/// element type (homogeneity enforced by `infer`/`type_errors`); DUPLICATES collapse at build. Lowers to
/// `Core::SetOf`. An empty `("set")` is a set of a deferred element type.
fn resolve_set(db: &Db, id: StructId) -> Resolved {
    let elems: std::rc::Rc<[StructId]> = db
        .ast
        .compound_form_of(id, CompoundCtor::Set)
        .unwrap_or(&[])
        .into();
    Resolved::Set { elems }
}

/// Resolve `(map (k v) …)` — a persistent key→value association literal. Each entry is a two-element
/// `(key value)` list; UNLIKE a record, BOTH positions are ORDINARY VALUE occurrences (resolved on
/// demand by the normal scope lookup, NOT read as a label via `read_key`) — that is what makes a map
/// key a VALUE (`(let ((a 5)) (map (a 1)))` keys by 5, `(+ 2 3)` is a runtime key, an unbound key is
/// the ordinary CDZ0101). A malformed entry (not a 2-element list — e.g. `(map ("a"))`, a key with no
/// value) is a `Poison` (CDZ0201), never a panic reaching for the absent value. An empty `(map)` is a
/// map with no entries. `infer`/`type_errors` enforce key/value homogeneity + duplicate-const-key.
fn resolve_map(db: &Db, id: StructId) -> Resolved {
    let tail = db
        .ast
        .compound_form_of(id, CompoundCtor::Map)
        .unwrap_or(&[]);
    let mut entries: Vec<(StructId, StructId)> = Vec::with_capacity(tail.len());
    for &entry in tail {
        // Prefer the canonical `(= key value)` FieldPair — map entries unify with record fields
        // (operator-ruled 2026-08-27: "prefer `=` for maps"; the `#map((= k v))` surface). The legacy
        // raw `(key value)` pair is still accepted through the migration (corpus migrates at M3).
        if let Some((k, v)) = db.ast.field_pair_parts(entry) {
            // Native FIELD_PAIR-leaf-headed entry (M2, what the reader now emits for #map entries).
            entries.push((k, v));
            continue;
        }
        if let Some((k, v)) = db.ast.field_pair(entry) {
            entries.push((k, v));
            continue;
        }
        match db.ast.get(entry) {
            // seq-276: a bare `(key value)` VALUE-map entry is NO LONGER accepted — require the canonical
            // FieldPair `(= key value)` (the `=` spellings were caught above). VALUE-scoped: this arm is in
            // `resolve_map` (value maps); the `map_pattern_of` pattern reader keeps the bare 2-element form.
            Struct::List(items) if items.len() == 2 => {
                return Resolved::Poison(
                    Reject::coded(
                        Code::Malformed,
                        "a map entry is a (key value) pair written `(= key value)` — a bare `(key value)` \
                         entry is no longer accepted (add the leading `=`)",
                    )
                    .at(entry),
                );
            }
            Struct::List(items) => {
                // A wrong-arity entry `(1 2 3)` / `(1)` — a fixed-arity shape (want 2). A SURPLUS element
                // gets the shared delete fix (`(1 2 3)` → `(1 2)`); too few is message-only. Anchored at
                // the offending entry.
                return Resolved::Poison(fixed_arity_reject(
                    entry,
                    items,
                    2,
                    "a map entry is a (key value) pair",
                ));
            }
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
        // Name the actual operand count + the canonical form, so the reader sees WHAT is wrong. TRAP: The
        // phrasing must NOT contain the substring "takes exactly" — that is `diag::EMIT_OPERAND_ARITY_MARKER`,
        // and `dedup_faults` DROPS a `Code::Malformed` fault matching it (+ "operand") as a redundant
        // emit-path operator-arity decline (the `Module.op … were given` consequent-suppression). A message
        // containing "takes exactly … operands" collided with that filter and was silently dropped for the
        // 0- and 3-operand cases (the 1-operand case slipped through by fault ordering) — a fragile,
        // content-keyed drop. Word around it: "is written `(: … )`; it needs a value and a type (N found)".
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!(
                "a type annotation is written `(: <expression> <type>)` — it needs a value and a type, \
                 but {} {} here",
                tail.len(),
                if tail.len() == 1 {
                    "part is"
                } else {
                    "parts are"
                }
            ),
        ));
    }
    Resolved::Annot {
        expr: tail[0],
        ty_expr: tail[1],
    }
}

/// Resolve `(const <expr>)` — the FORCE-EVAL block (operator's const-demand construct). An EXPRESSION
/// form with exactly ONE operand, distinct from the `(const (: d T))` PARAM modifier (`strip_const_params`
/// unwraps that at load and never touches a body, so a `(const …)` reaching here is the block). SEE-THROUGH
/// like `resolve_annot` — carries the inner `expr` so typing/lowering follow it; the fold-or-reject
/// force-eval is downstream in lowering (v-compiler-primitives). A wrong operand count is a coded reject.
fn resolve_const_block(db: &Db, id: StructId) -> Resolved {
    let tail = db.ast.as_form(id, "const").unwrap_or(&[]);
    if tail.len() != 1 {
        return Resolved::Poison(Reject::coded(
            Code::Malformed,
            format!(
                "a `const` block is written `(const <expression>)` — it needs exactly one expression, \
                 but {} parts are here",
                tail.len()
            ),
        ));
    }
    Resolved::ConstBlock { expr: tail[0] }
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
    fn resolved_ref_agrees_with_resolved_of_on_every_node() {
        // `resolved_ref` is the zero-clone borrow companion of `resolved_of` (used on the hot reducer
        // path). It MUST return exactly what `resolved_of` does for every node — same variant, same
        // payload — whether the memo is cold (first touch) or warm. Pin that invariant over a program
        // exercising several resolved shapes (Ref, Apply, Record via a module, Member, Let), reading each
        // node COLD through `resolved_ref` first (so it drives the compute+fill path), then confirming
        // `resolved_of` returns an equal value.
        let ast = parse(
            "(module m (def (main) (do (module inner (def (answer) 42)) \
               (let ((v (. inner answer))) (+ (v unit) 1)))) (export main))",
        );
        // Two DBs from the same source so the two queries run against independent memo columns: one driven
        // ref-first, one of-first — and cross-check they agree node-for-node.
        let mut db_ref = Db::load(ast.clone());
        let mut db_of = Db::load(ast);
        for i in 0..db_ref.ast.structure.len() as u32 {
            let id = StructId(i);
            // COLD ref read (fills db_ref's memo), cloned to compare after releasing the borrow.
            let via_ref = resolved_ref(&mut db_ref, id).clone();
            let via_of = resolved_of(&mut db_of, id);
            assert_eq!(
                format!("{via_ref:?}"),
                format!("{via_of:?}"),
                "resolved_ref and resolved_of disagree at node {i}"
            );
        }
        // And a WARM ref read (memo already filled) must still equal the of-value — the hot path.
        for i in 0..db_ref.ast.structure.len() as u32 {
            let id = StructId(i);
            let warm = resolved_ref(&mut db_ref, id).clone();
            let of = resolved_of(&mut db_ref, id);
            assert_eq!(
                format!("{warm:?}"),
                format!("{of:?}"),
                "warm ref disagrees at {i}"
            );
        }
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

    /// The `StructId` of the first bare-name occurrence spelled `name` in `db` — for driving a name
    /// resolution assertion.
    fn name_node(db: &Db, name: &str) -> StructId {
        (0..db.ast.structure.len() as u32)
            .map(StructId)
            .find(|&id| db.ast.as_name(id) == Some(name))
            .unwrap_or_else(|| panic!("a `{name}` node"))
    }

    #[test]
    fn a_top_level_module_name_resolves_from_a_top_level_def_body() {
        // The reported bug: a `(module Temp …)` that is a top-level `(do …)` element — a sibling of the
        // top-level defs — binds `Temp` program-wide, so `main`'s body `(. Temp c-to-f)` must resolve
        // `Temp` to the module's synth record. Before the fix this was CDZ0101 `unbound name Temp`: the
        // scope walk stops at the root `do` (which binds nothing) and no `resolve_name` step consulted the
        // module set. `Temp` (the member-access head) must now resolve to a `Ref` to the synth record.
        let ast = parse(
            "(do (module Temp (def (c-to-f c) (+ (/ (* c 9) 5) 32)) (export c-to-f)) \
             (def (main) ((. Temp c-to-f) 100)) (export main))",
        );
        let mut db = Db::load(ast);
        // The `Temp` occurrence heading `(. Temp c-to-f)` is the LAST `Temp` node (the module decl's head
        // is the first); assert the member-access head resolves to the module's synth record.
        let synth = db
            .top_level_module_by_name("Temp")
            .expect("Temp registered as a top-level module");
        let head = (0..db.ast.structure.len() as u32)
            .map(StructId)
            .filter(|&id| db.ast.as_name(id) == Some("Temp"))
            .find(|&id| {
                // A member-access head: its parent is a `(. Temp …)` form with `Temp` at child index 1.
                db.parent_of(id)
                    .and_then(|p| db.ast.as_form(p, "."))
                    .and_then(|t| t.first().copied())
                    == Some(id)
            })
            .expect("a `(. Temp …)` head");
        assert_eq!(
            resolved_of(&mut db, head),
            Resolved::Ref { value: synth },
            "the member-access head `Temp` must resolve to the module's synth record"
        );
    }

    #[test]
    fn a_do_local_module_is_not_leaked_program_wide() {
        // A do-LOCAL module (nested inside a def body's `(do …)`) is lexically scoped — it must NOT be
        // surfaced by `top_level_module_by_name`, which consults only top-level (root-child) modules. A
        // reference to `m` from OUTSIDE its enclosing `do` would then be unbound (its own body resolves it
        // via the lexical scope walk, tested by the corpus). This pins that the top-level step does not
        // leak a nested module.
        let ast = parse(
            "(do (def (main) (do (module m (def (answer) 42)) ((. m answer) unit))) (export main))",
        );
        let db = Db::load(ast);
        assert_eq!(
            db.top_level_module_by_name("m"),
            None,
            "a do-local module must not resolve as a top-level module"
        );
        // Sanity: the do-local module IS registered (just not as a top-level one).
        let _ = name_node(&db, "m");
        assert!(
            db.modules.iter().any(|md| md.name == "m"),
            "the do-local module is still registered in db.modules"
        );
    }

    /// The field labels of the record `m` resolves to — a helper for the export-visibility tests. Panics
    /// if `m` does not resolve to a `Resolved::Record`.
    fn module_record_fields(db: &mut Db, module_name: &str) -> Vec<String> {
        let name = name_node(db, module_name);
        match resolved_of(db, name) {
            Resolved::Ref { value } => match resolved_of(db, value) {
                Resolved::Record { fields } => fields.keys().map(|s| s.name.to_string()).collect(),
                other => panic!("module `{module_name}` record is not a Record: {other:?}"),
            },
            other => panic!("module `{module_name}` did not resolve to a Ref: {other:?}"),
        }
    }

    #[test]
    fn an_export_clause_filters_the_module_record_to_the_named_members() {
        // Visibility is explicit: a module's `(export pub)` clause makes `pub` a field of the record and
        // leaves the unnamed `secret` OUT — so `(. m secret)` is a closed-record CDZ0201 downstream. Assert
        // the synth record carries `pub` and NOT `secret`.
        let ast = parse(
            "(do (module m (def (pub x) (+ x 1)) (def (secret x) (+ x 100)) (export pub)) \
             (def (main) ((. m pub) 5)) (export main))",
        );
        let mut db = Db::load(ast);
        let fields = module_record_fields(&mut db, "m");
        assert!(
            fields.contains(&"pub".to_string()),
            "pub is exported: {fields:?}"
        );
        assert!(
            !fields.contains(&"secret".to_string()),
            "secret is NOT exported and must be absent from the record: {fields:?}"
        );
    }

    #[test]
    fn no_export_clause_exports_every_member() {
        // The export-EVERYTHING default: with NO `(export …)` clause the record carries every definition,
        // the pre-existing corpus behavior. Assert both `a` and `b` are present.
        let ast = parse(
            "(do (module m (def (a x) (+ x 1)) (def (b x) (+ x 2))) \
             (def (main) ((. m a) 1)) (export main))",
        );
        let mut db = Db::load(ast);
        let fields = module_record_fields(&mut db, "m");
        assert!(fields.contains(&"a".to_string()), "a present: {fields:?}");
        assert!(fields.contains(&"b".to_string()), "b present: {fields:?}");
    }

    #[test]
    fn a_private_module_member_body_sees_an_exported_sibling() {
        // The false-CDZ0101 the privacy landing left: `even` is exported, `odd` is private (no synth field
        // built for it), and the two are mutually recursive. `odd`'s body's `(even …)` call must resolve —
        // `odd`'s body ascends through the `(module …)` FORM (never reparented under a synth record), so the
        // form itself must resolve siblings (`module_form_sibling_binds` / `binder_in` Case R2). Before the
        // fix this reference was `Resolved::Poison` (unbound name), the reported bug.
        let ast = parse(
            "(do (module m (export even) \
             (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1)))) \
             (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1))))) \
             (def (main) ((. m even) 4)) (export main))",
        );
        let mut db = Db::load(ast);
        // The `even` reference INSIDE `odd`'s body: an `even` name occurrence that HEADS an application
        // `(even (- n 1))` (parent is a list with `even` at child index 0) whose nearest enclosing `(def
        // (odd …) …)` is `odd`'s. There are two such call heads (`odd`'s call to `even`, and `even`'s own
        // if-body); the one under `odd` is the one at issue — but BOTH must resolve, so assert every
        // application-head `even` reference resolves (a private-body ref and an exported-body ref alike).
        let heads: Vec<StructId> = (0..db.ast.structure.len() as u32)
            .map(StructId)
            .filter(|&id| db.ast.as_name(id) == Some("even"))
            .filter(|&id| {
                db.parent_of(id).and_then(|p| match db.ast.get(p) {
                    Struct::List(children) => children.first().copied(),
                    _ => None,
                }) == Some(id)
            })
            .collect();
        assert!(
            !heads.is_empty(),
            "expected at least one `even` application head"
        );
        for head in heads {
            assert!(
                !matches!(resolved_of(&mut db, head), Resolved::Poison(_)),
                "an `even` reference (including from private `odd`'s body) must resolve, not poison"
            );
        }
    }
}
