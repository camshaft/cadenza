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
use crate::ast::{Arenas, Leaf, LeafId, Struct, StructId};
use crate::core::Core;
use crate::resolved::Resolved;
use crate::ty::Ty;
use std::collections::BTreeMap;

/// A top-level definition located by the one cheap top-level scan: its name, its parameter
/// occurrences (empty = nullary), and its body occurrence (absent = malformed). The body is LOCATED,
/// never entered by the scan — entering it is a later per-node demand.
#[derive(Clone, PartialEq, Debug)]
pub struct Def {
    pub name: String,
    /// Signature occurrence, for diagnostics.
    pub sig_occ: StructId,
    /// Parameter occurrences, in signature order (empty = a nullary def). A parameter may be a bare
    /// name or an annotated `(: name T)` binder.
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
    /// The `(export NAME)` clause occurrence, for a diagnostic to anchor to (e.g. a duplicate export).
    pub occ: StructId,
}

/// One variant of a sum declaration — a `(name payload-type…)` list or a bare nullary name. Its
/// position in [`TypeDecl::variants`] is its DISCRIMINANT (declaration order).
#[derive(Clone, PartialEq, Debug)]
pub struct Variant {
    /// The variant's declared name (`Some` in `(Some Int64)`).
    pub name: String,
    /// The variant's NAME occurrence — the head of its `(name payload…)` list, or the bare atom for a
    /// nullary variant. A duplicate-variant diagnostic anchors here.
    pub name_occ: StructId,
    /// The variant's PAYLOAD TYPE occurrences, in declaration order (empty for a nullary variant). These
    /// are the user-written type expressions after the name in `(name payload…)`; the sum-record
    /// synthesis copies them into each constructor's arrow type `(-> payload… Sum)`.
    pub payloads: Vec<StructId>,
}

/// A `(type NAME variant…)` sum-type declaration scanned from the top level. Each variant is a
/// `(name payload-type…)` list or a bare nullary name; a sum's variant names are a fixed SET (like a
/// record's fields), so a duplicate is ill-formed (CDZ0201). Stage: the declaration is scanned, its
/// variant set validated, and its record synthesized (`sums::synthesize`) so the type name + variant
/// constructors resolve; construction/match over the sum are later sub-increments.
#[derive(Clone, PartialEq, Debug)]
pub struct TypeDecl {
    /// The type's name (`T` in `(type T …)`).
    pub name: String,
    /// The declaration occurrence — the NOMINAL IDENTITY of the sum (`type-system.md §158`, realized
    /// as the arena occurrence) and the anchor for a declaration-level diagnostic. Because identity is
    /// this occurrence and NOT the name, two `(type Foo …)` declared separately are DISTINCT types even
    /// with the same name — the property no name-keyed map could preserve.
    pub occ: StructId,
    /// The sum's TYPE PARAMETERS, in first-appearance order — the IMPLICIT generics
    /// (`type-system.md §Generics Are Type-Valued Parameters`). A free LOWERCASE name in any variant
    /// payload is a type parameter (types are Capitalized — `Int64`, `Option`; a lowercase payload name
    /// is a variable, as the prelude operator type-lambdas already use lowercase `a`). Empty for a
    /// MONOMORPHIC sum (`(type Sign Neg Zero Pos)`). `(type Option (Some a) None)` has `params: ["a"]`;
    /// `(type Result (Ok a) (Err b))` has `["a", "b"]`. Order fixes the positional `Ty::Sum::args`.
    pub params: Vec<String>,
    /// The variants in declaration order — each position's index is its discriminant.
    pub variants: Vec<Variant>,
    /// The SYNTHESIZED record occurrence — the record (`crate::sums`) whose `(meta t)` is this sum's
    /// type-value and whose fields are its variant constructors. `None` until `sums::synthesize` runs
    /// during `Db::load`; ALWAYS `Some` once the `Db` exists. This is what the type NAME resolves to (a
    /// `Ref` to it), reached by the ordinary scope→top-level lookup like a `def` — no separate name map
    /// (which would collapse identity onto the name and clobber a same-named declaration).
    pub synth: Option<StructId>,
}

/// The bound on β-reduction nesting depth — a backstop. A recursive function is caught STATICALLY
/// (a call whose callee transitively references itself declines before any inlining — see
/// `eval::is_recursive`), so this only guards a non-recursive fold that nests unexpectedly deep. Kept
/// low: a recursive function that branches (several self-calls per body) would explode EXPONENTIALLY
/// in node appends well before a high depth, so the static check is the real guard and this is a
/// cheap safety net. No legitimate compile-time fold nests this deep.
pub(crate) const REDUCE_DEPTH_LIMIT: u32 = 32;

/// The bound on RECURSIVE-DESCENT depth across the demand queries (`type_of`, the fault `collect`, and
/// `core_of`) — a backstop against a native stack overflow on pathologically deep input. Each query is
/// recursive descent (a node's answer re-enters the query for its sub-expressions), so a deeply NESTED
/// expression (`(+ 1 (+ 1 …))` thousands deep) or an unproductive self-recursion a nullary call
/// re-enters (`(def (f) (f))`) would recurse until the process aborts. Past this depth the query
/// DECLINES (a resource-limit poison / `Any` type) rather than crashing — a compiler must never abort
/// on well-formed input (`self-hosting-and-bootstrap.md` §An Unsupported Construct Is Declined, Not
/// Miscompiled). Set well above any real nesting depth (a program nesting expressions this deep is not
/// hand-written) and well below the native stack's ceiling (~a few thousand frames per query), so a
/// legitimate program never hits it and a pathological one declines long before the abort. Mirrors
/// [`REDUCE_DEPTH_LIMIT`] for the evaluator's own recursion.
pub(crate) const DESCENT_DEPTH_LIMIT: u32 = 1024;

/// An RAII guard for one active β-reduction: holds the reduction depth bumped for its lifetime and
/// decrements it on drop, so the depth exactly reflects the reductions currently on the stack. Held by
/// the evaluator across the reduce-and-evaluate of a lambda application.
pub struct ReductionGuard<'a> {
    db: &'a mut Db,
}

impl ReductionGuard<'_> {
    /// The database, to continue reducing within this guarded scope.
    pub fn db(&mut self) -> &mut Db {
        self.db
    }
}

impl Drop for ReductionGuard<'_> {
    fn drop(&mut self) {
        self.db.reduce_depth = self.db.reduce_depth.saturating_sub(1);
    }
}

/// The memo key for a built constructor value: the primitive applied plus its argument VALUES (not
/// occurrences), so two `(Int 64)` written at different source occurrences — or the same occurrence
/// demanded repeatedly — share one built module. `i64` args cover the width of `(Int W)`; the key
/// widens as constructors take other argument kinds.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BuildKey {
    pub prim: crate::resolved::Prim,
    pub args: Vec<i64>,
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
    /// The top-level `(type …)` sum declarations, from the scan (their variant sets validated at
    /// well-formedness time — a duplicate variant name is CDZ0201).
    pub type_decls: Vec<TypeDecl>,

    /// For each `StructId`, the `List` occurrence that holds it as a child, or `None` for the root.
    /// The structure arena is NOT deduplicated, so every occurrence has exactly one parent — this is
    /// one deterministic scan, filled at load. It is what lets `resolve` derive a name's LEXICAL scope
    /// from the node's POSITION (walking parents to the nearest enclosing binder) rather than
    /// threading a scope argument that would break per-`StructId` memoization — the same
    /// provenance-by-back-reference the columns model uses for source position.
    parent: Vec<Option<StructId>>,

    /// For each `StructId`, its POSITION among its parent's children (`0` for a root or a node with no
    /// recorded position — meaningful only when the node has a parent). Built in the SAME single pass
    /// as `parent`. It turns "which child of my parent am I?" — the window bound a scope walk needs
    /// when it ascends INTO a `let` bindings-list from one binding pair — into an O(1) read rather than
    /// an O(k) `position()` scan of the sibling list. Without it, resolving the n references of a
    /// sequential `let` (`(let ((a …)(b (f a))(c (g b))…) …)`) re-scans an ever-longer sibling prefix
    /// per reference — O(n²); with it, each ascent is O(1).
    child_ix: Vec<u32>,

    /// For each def BODY occurrence, its index in [`defs`]. Built once at load from the top-level scan.
    /// It answers "is this occurrence a def's body, and which def?" — the question the recursion
    /// analysis asks of EVERY callee edge (`eval::callee_body`) and lowering/inference ask per call — in
    /// O(1), replacing an `db.defs.iter().any(|d| d.body == Some(v))` / `.position(…)` LINEAR scan. On a
    /// def-call chain `g0→g1→…→gN` the recursion walk traverses O(N) edges for each of N bodies and each
    /// edge did that O(N) scan — O(N³); with this map each edge is O(1), so the walk is O(N²) and, being
    /// memoized per body in `recursive`, the whole compile is O(N). A def with no body (malformed) or a
    /// synthesized body contributes no entry.
    ///
    /// [`defs`]: Db::defs
    def_by_body: crate::fxhash::FxHashMap<StructId, usize>,

    /// For each def NAME, the index in [`defs`] of the FIRST def with that name. Built once at load.
    /// `resolve_name` consults it for EVERY non-local, non-parameter name reference (before the prelude
    /// fallback) — so a program with N defs, each body referencing an operator or a sibling, made N
    /// lookups, and the old linear `defs.iter().position(|d| d.name == name)` turned that into O(N²)
    /// (measured: an N-def program went superlinear, 16000 defs = 289ms). This map makes each lookup
    /// O(1). FIRST-wins matches the old scan's first-match (a duplicate def name keeps the earlier def;
    /// the well-formedness pass reports the duplicate separately).
    ///
    /// This is a pure lookup ACCELERATOR over `defs`, NOT a source of truth: a def's identity is its
    /// occurrence (its `StructId`), exactly as for `type_decls` — the index only speeds "which def has
    /// this name?", it never decides meaning (`prelude-and-resolution.md`; the operator's rule "a
    /// program-written binding is keyed by OCCURRENCE, never its own name map").
    ///
    /// ⚠ FLAT-NAMESPACE ASSUMPTION — revisit when nested modules / imports land. `scan_top_level` is
    /// flat today (`top_items` does NOT recurse into a nested `(module …)`), so ALL defs share one
    /// namespace and a bare `String` key is correct. Under nesting, two same-named defs in sibling
    /// submodules are still DISTINCT (different occurrences) but would COLLIDE in this map. The fix is
    /// to key by (enclosing-module occurrence, name) and have the lookup walk enclosing modules outward
    /// — the module-scope analog of `lookup_scope`'s lexical parent-walk (module scope = one more rung
    /// of the same ladder; resolution's scope→defs→sums→prelude ORDER is unchanged, only step 2's
    /// lookup gains the scope argument). Do NOT grow this into a global `name → occurrence` map (the
    /// `db.sum_types` mistake). The `same_named_defs_are_distinct_bindings` test pins the invariant.
    ///
    /// [`defs`]: Db::defs
    def_name_index: crate::fxhash::FxHashMap<String, usize>,

    /// Per-SCOPE-FORM parameter binder index: `(scope_form_occ, param_name) → the param's NAME
    /// occurrence`. Covers the two forms whose parameters bind by a signature scan — a `fn`'s param
    /// list and a `def`'s signature. Built once at load ([`build_scope_binders`]).
    ///
    /// `binder_in`'s Case-3/Case-4 answered "does this scope declare `name`, and where?" by scanning
    /// ALL parameters of the signature (to honour last-wins shadowing) on EVERY body reference — so a
    /// body with N references into an N-parameter signature (a wide `(def (f p0 … pN) (+ p0 (+ p1 …)))`
    /// or a wide record/tuple over the params) was O(N²). This map makes each lookup O(1). LAST-wins is
    /// baked in at build time (a later param of the same name overwrites the earlier entry), matching
    /// the old forward scan's "last assignment wins". Keyed by the SCOPE FORM's occurrence, so distinct
    /// signatures never collide (the per-scope analog of `def_name_index`, and it composes with
    /// `lookup_scope`'s parent-walk unchanged — only the innermost per-form probe is now O(1)).
    ///
    /// Nested (`scope → name → occ`) rather than a `(scope, name)` tuple key so the inner lookup can
    /// borrow the reference's `&str` directly (via `String: Borrow<str>`) — no per-lookup `String`
    /// allocation on the hot resolve path.
    scope_binders: crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashMap<String, StructId>>,

    /// Per-node LEXICAL-SCOPE SKIP pointer: for each `StructId`, the nearest STRICT ANCESTOR that is a
    /// binding-CANDIDATE (a form `resolve::binder_in` could bind a name in — a `let`/`fn`/`def`, a let
    /// bindings-list, or a match arm), paired with that candidate's DIRECT CHILD on the path down to
    /// this node (the `from` argument `binder_in` needs to decide its window: body vs bindings-list
    /// pair, match-arm body). `None` once the walk reaches the root with no candidate above.
    ///
    /// `lookup_scope` walks parents to find a name's binder, but ascends EVERY enclosing form — and in a
    /// deeply nested value / expression (`(record … (next (record …)))`, a deep `if`/`let` nest) the
    /// vast majority are NON-binding forms that bind nothing, so an N-deep reference visited O(depth)
    /// no-op ancestors → O(N²) across N references (the deepdata/deeplet/bignest family; `as_form` +
    /// `resolved_of` were ~all of self-time). This pointer lets the walk HOP candidate-to-candidate,
    /// skipping the non-binding spine — O(1) per hop, so a reference costs only the number of enclosing
    /// BINDERS, not the raw nesting depth. A shape with few binders (the common case) resolves in O(1).
    ///
    /// Built once at load ([`build_scope_skip`]) by path-compression over `parent` (no id-ordering
    /// assumption), and extended incrementally in [`push_list`] for β-reduction's synthesized forms
    /// (a load-time-only arena index would miss a copied lambda/let body — the lesson from the
    /// per-scope binder index). A node with no recorded entry falls back to the plain parent walk.
    scope_skip: Vec<Option<(StructId, StructId)>>,

    /// The prelude — the one map of built-in bindings, installed ONCE at load as ordinary AST nodes
    /// (a built-in module is just a record; see `crate::prelude`). Maps a built-in name to the arena
    /// occurrence it binds to. `resolve` consults it after the lexical scope, so a program binding
    /// shadows a built-in by the ordinary lookup precedence.
    pub prelude: BTreeMap<String, StructId>,

    /// The evaluator's current β-reduction nesting depth. β-reduction inlines a callee's body; a
    /// TERMINATING fold bottoms out at a bounded depth, while a RECURSIVE function inlines without end.
    /// So a bounded depth is the sound guard: past [`REDUCE_DEPTH_LIMIT`] the evaluator declines rather
    /// than diverging (`reference-compiler.md` §The Evaluator Bounds Its Own Reduction And Declines
    /// Rather Than Diverges). (A body-on-the-stack set is NOT a sound recursion signal — `(f (f v))`
    /// legitimately nests the same body twice while both calls terminate; only the depth distinguishes
    /// a terminating nest from an unbounded one.)
    pub(crate) reduce_depth: u32,

    /// The current RECURSIVE-DESCENT depth across the demand queries (`type_of`, `collect`, `core_of`)
    /// — the recursive-descent backstop. Bumped on entering a query's recursion and restored on exit;
    /// past [`DESCENT_DEPTH_LIMIT`] the query declines instead of recursing, so pathologically deep
    /// input (or an unproductive self-recursion) yields a resource-limit decline rather than a native
    /// stack overflow. Shared across the queries, which interleave (`collect` calls `type_of`), so the
    /// counter reflects true stack depth; it returns to 0 between top-level demands. See
    /// [`DESCENT_DEPTH_LIMIT`].
    pub(crate) descent_depth: u32,

    /// Memo of built values the evaluator produced by applying a native constructor — keyed by the
    /// reduction `(prim, arg-values)`, mapping to the built node's occurrence. Without it, a
    /// type-constructor application like `(Int 64)` would append a FRESH module on every demand
    /// (`type_of`, `core_of`, each projection re-runs the reduction) and again for every source
    /// occurrence — bloating the arena and, worse, making the reduction non-idempotent (a node's fact
    /// must be stable across demands). The first demand builds and caches; every later one returns the
    /// same node, so `(Int 64)` denotes ONE module however many times it is written or demanded.
    pub(crate) build_cache: crate::fxhash::FxHashMap<BuildKey, StructId>,

    /// Memo of the static recursion analysis (`eval::is_recursive`): for a function BODY occurrence,
    /// whether that function is (transitively) recursive — a call whose callee reaches the same body.
    /// A pure function of the fixed def/`let` structure, so it caches by body `StructId` like
    /// `build_cache`. The evaluator reads it BEFORE β-reducing a call and DECLINES a recursive one (a
    /// recursive function cannot be inlined to a normal form at compile time — it needs runtime
    /// specialization), which is what keeps a self-calling function from inlining without end / from
    /// exploding exponentially in appended nodes when its body branches.
    pub(crate) recursive: crate::fxhash::FxHashMap<StructId, bool>,

    /// Memo of one function body's DIRECT callee edges (`eval::collect_callees`): for a body/callee
    /// occurrence, the list of callee bodies its code calls (excluding nested `fn` boundaries). A pure
    /// function of the fixed resolved structure — the same node's edges never change — so it caches by
    /// occurrence `StructId` like `recursive`. The recursion DFS (`is_recursive`) expands the SAME
    /// bodies once per ancestor that reaches them; without this memo each expansion re-runs
    /// `collect_callees`, which re-`resolved_of`s the whole subtree and clones a `Resolved` (owning a
    /// `Vec`) per node — an O(N²) clone/drop churn on a def-call chain. Memoized, each body's edges are
    /// computed once and the DFS just reads the cached slice.
    pub(crate) callee_edges: crate::fxhash::FxHashMap<StructId, Vec<StructId>>,

    /// Memo of an operator's `(meta t)` reduced to a CANONICAL [`Scheme`] (`eval::scheme_of`), keyed by
    /// the `(meta t)` NODE — which is SHARED across every application of the same operator (all `+`
    /// occurrences project the one prelude `+`'s type-lambda). Reading it re-reduces the type-lambda via
    /// `type_in_env` (a ~half-the-time cost on operator-heavy code); the reduction is a pure function of
    /// the fixed `(meta t)` structure, so it caches. The cached scheme uses canonical (from-0) variable
    /// numbering; every caller `instantiate`s it — which renames the bound variables to truly-fresh ones
    /// — so a shared canonical scheme is correct for all uses. `None` caches "has no scheme". Mirrors
    /// `recursive`/`callee_edges` (a pure fact keyed by node identity).
    pub(crate) scheme_cache: crate::fxhash::FxHashMap<StructId, Option<crate::ty::Scheme>>,

    /// Memo for the wasm backend's `mutual_loop_group(self_def)` — the tail-recursive SCC a def compiles
    /// its shared loop over. `select_function_of` computes it for EVERY def, and the computation is a
    /// double BFS over tail-callees (forward-reach + reach-back-to-self per member), so N mutually
    /// tail-recursive same-signature defs cost O(N²) EACH → O(N³) over the group (measured: 200 mutual
    /// defs = 687ms, ~O(N³), with O(N²)-size wasm). The group is a pure function of the call graph +
    /// signatures (fixed), keyed by the def's own index (the ordering is `self_def`-first, so it differs
    /// per member — cache each member's own view). Caching collapses the per-def recompute; a def not in
    /// any loop caches an empty vec. Keyed by def index (a `usize` into `defs`).
    pub(crate) mutual_loop_cache: crate::fxhash::FxHashMap<usize, Vec<usize>>,

    /// Reusable SCRATCH buffers for the recursion walk (`eval::is_recursive`) — the visited set and the
    /// worklist of its iterative call-graph DFS. Held here (not allocated per call) so the walk churns
    /// no ephemeral collections: `is_recursive` takes them out (`mem::take`), CLEARS them, uses them,
    /// and returns them. Their contents are meaningless between calls — this is workspace, not state,
    /// the one exception to the "columns are the state" rule, justified by the allocation it saves in a
    /// walk that runs once per function body.
    pub(crate) rec_visited: crate::fxhash::FxHashSet<StructId>,
    pub(crate) rec_worklist: Vec<StructId>,

    /// Memo of a top-level definition's generalized SIGNATURE as a [`crate::ty::Scheme`] — its type as
    /// a value, so a CALL can be typed by instantiating the scheme rather than β-reducing the body
    /// (`infer::def_scheme`). Keyed by the `db.defs` index. An unannotated recursive def's parameters
    /// are pinned by the connected solve (`solve_recursive_params`, A2), so its scheme is `Some`;
    /// `None` remains only for a def with a genuinely undetermined parameter (no use constrained it —
    /// it grounds to `Any`), where the caller falls back to β-reduction. A pure function of the fixed
    /// def structure, so it caches like `build_cache`.
    pub(crate) def_schemes: crate::fxhash::FxHashMap<usize, Option<crate::ty::Scheme>>,

    /// Memo of an UNANNOTATED RECURSIVE def's parameter types, solved by the connected def-body solve
    /// (`infer::solve_recursive_params` — ANF step 2 / A2). Keyed by the parameter's NAME occurrence
    /// (the `Param` binder). A recursive def cannot inline, so its parameters need machine types to
    /// cross to a wasm function; unlike an annotated param (whose type is its annotation) or a
    /// non-recursive one (which inlines at its call site), a recursive unannotated param's type is
    /// INFERRED from its uses in the body + call sites by a single threaded-`Subst` solve — the one
    /// place inference is a connected solve rather than the per-node column read. `type_of` of such a
    /// `Param` reads this map (triggering the solve on first demand); the solve fills every parameter of
    /// the def at once. Order-independent (unification is), so the two ends agree regardless of demand
    /// order. A pure function of the fixed def structure, so it caches like `def_schemes`.
    pub(crate) param_types: crate::fxhash::FxHashMap<StructId, crate::ty::Ty>,

    /// Guard against re-entering the recursive-parameter solve for a def already being solved — the
    /// solve types the body with a LOCAL env (not `type_of`), so a self-call reads the provisional
    /// signature rather than re-triggering; this set is the defensive backstop that a demand landing
    /// mid-solve returns without recomputing. Holds the def indices whose solve is on the stack.
    pub(crate) solving_params: crate::fxhash::FxHashSet<usize>,

    /// The set of `let`-binding INITIALIZER occurrences that `lower` decided to KEEP as an A-normal
    /// `Core::Let` binding — a runtime value used more than once, named once so it is computed once
    /// (`reference-compiler.md` §The Core Representation Is In A-Normal Form). Populated while lowering
    /// the enclosing `let` (before its body's references are lowered), then read when lowering a
    /// `Resolved::Ref`: a ref to an init in this set lowers to a `Core::LocalRef` (read the shared
    /// slot) rather than following through to the value's core (which would recompute it). A binding
    /// NOT in this set is copy-propagated / erased — the admin-redex elimination that keeps naming
    /// free (`reference-compiler.md` ¶3), so a single-use or constant binding leaves no `Let` and the
    /// emitted bytes are unchanged. It is a memo of a lowering decision (like `build_cache`), not a
    /// column: the decision is a pure function of the fixed `let` structure, so recording it once and
    /// reading it at each reference keeps the two ends of one binding in agreement.
    pub(crate) kept_bindings: crate::fxhash::FxHashSet<StructId>,

    /// The set of subtree roots [`crate::resolve::resolve_subtree`] has ALREADY fully walked. That
    /// function eagerly resolves every node under a root to PIN an argument's meaning before
    /// β-reduction re-parents it; `apply_lambda` calls it on each argument at EVERY application. The
    /// per-node `resolved_of` is memoized, but the WALK itself (descend + clone children + recurse)
    /// was not — so re-pinning the same (or an overlapping, growing) subtree across a chain of
    /// applications re-walked it O(depth) times, O(N²) on a shared-tuple projection chain
    /// (`(+ (. t 0) (+ (. t 1) …))`). Recording a fully-walked root here makes `resolve_subtree`
    /// idempotent-cheap: a second call on an already-walked root returns immediately. A pure function
    /// of the fixed AST (a subtree, once resolved, stays resolved), like the other memos.
    pub(crate) resolved_subtrees: crate::fxhash::FxHashSet<StructId>,

    /// The number of structure nodes in the DECODED PROGRAM — the count before the prelude and any
    /// evaluator-synthesized nodes were appended. A `StructId` below this is a genuine user-program
    /// node the front-end's span table is keyed by; a `StructId` at or above it is a PRELUDE node (a
    /// built-in binding) or an evaluator-SYNTHESIZED node (a β-reduced body, a built `(Int W)` module)
    /// — neither has a source position. A diagnostic must only ever report an origin below this bound
    /// (see [`Db::is_user_node`]); reporting a prelude/synthesized id would map to garbage (or nothing)
    /// in the consumer's span table.
    user_node_count: u32,

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
        let mut ast = ast;
        // The program's node count, captured BEFORE the prelude appends — the boundary between user
        // nodes (which the front-end's span table covers) and everything appended after. Ids `0..this`
        // are the user program; ids at/above are prelude or evaluator-synthesized.
        let user_node_count = ast.structure.len() as u32;
        // Install the prelude as ordinary AST nodes FIRST, so its records get `StructId`s (after the
        // program's — no program id shifts) and the parent index covers them too. A built-in module is
        // just a record in the arena; the prelude map is `name → its occurrence`.
        let mut prelude = crate::prelude::install(&mut ast);
        let (defs, exports, mut type_decls) = scan_top_level(&ast);
        // Append the BUILT-IN sum declarations (generic `Option`/`Result`) as ordinary `TypeDecl`s, so a
        // program uses bare `Some`/`None`/`Ok`/`Err` + `Option`/`Result` without declaring them (the
        // corpus surface). They scan exactly like a user declaration; a user `(type Option …)` shadows
        // (top-level `type_decls` resolve before the prelude). Appended AFTER the user scan so a user
        // declaration takes priority in `type_decl_by_name` (first-wins).
        let prelude_sum_count = {
            let ps = crate::sums::prelude_decls(&mut ast);
            let n = ps.len();
            type_decls.extend(ps);
            n
        };
        // Synthesize each `(type …)` sum as a record (fields = variants), appending its nodes to the
        // arena and recording each on its `TypeDecl.synth` — AFTER the scan (it reads `type_decls`) and
        // BEFORE the parent index (which must index the synthesized nodes so a name inside a synthesized
        // ctor type resolves by the scope walk).
        crate::sums::synthesize(&mut ast, &mut type_decls);
        // Bind the built-in sums' names in the PRELUDE map (the last-consulted lookup): the sum name to
        // its record (a type constructor / type-value), each variant name BARE to its ctor field. So a
        // reference to `Some`/`None`/`Ok`/`Err`/`Option`/`Result` resolves through the ordinary
        // scope→def→prelude order — a user binding of the same name shadows.
        for decl in type_decls.iter().skip(type_decls.len() - prelude_sum_count) {
            if let Some(record) = decl.synth {
                prelude.entry(decl.name.clone()).or_insert(record);
                for variant in &decl.variants {
                    if let Some(field) =
                        crate::sums::variant_ctor_field(&ast, record, &variant.name)
                    {
                        prelude.entry(variant.name.clone()).or_insert(field);
                    }
                }
            }
        }
        let (parent, child_ix) = parent_index(&ast);
        // Per-node lexical-scope skip pointer (nearest binding-candidate ancestor + entry child) so the
        // scope walk hops over non-binding forms — O(1) per binder instead of O(nesting depth).
        let scope_skip = build_scope_skip(&ast, &parent);
        // Index each SCOPE FORM's parameter binders by name (last-wins), so `binder_in`'s per-reference
        // "does this scope declare `name`?" probe is O(1) rather than an O(params) signature scan.
        let scope_binders = build_scope_binders(&ast);
        // Index each def by its body occurrence — the reverse of `defs[i].body`, so a "which def owns
        // this body?" lookup is O(1) rather than a linear scan of `defs`. A def with no body (malformed)
        // contributes no entry; a body occurrence is unique to one def, so no collision.
        let mut def_by_body = crate::fxhash::FxHashMap::default();
        // Index each def by NAME too (first-wins) — so `resolve_name`'s per-reference lookup is O(1),
        // not an O(defs) scan (which made name resolution O(N²) on a many-def program).
        let mut def_by_name = crate::fxhash::FxHashMap::default();
        for (i, d) in defs.iter().enumerate() {
            if let Some(body) = d.body {
                def_by_body.insert(body, i);
            }
            def_by_name.entry(d.name.clone()).or_insert(i);
        }
        Db {
            ast,
            defs,
            exports,
            type_decls,
            parent,
            child_ix,
            scope_skip,
            def_by_body,
            def_name_index: def_by_name,
            scope_binders,
            prelude,
            user_node_count,
            reduce_depth: 0,
            descent_depth: 0,
            build_cache: crate::fxhash::FxHashMap::default(),
            recursive: crate::fxhash::FxHashMap::default(),
            callee_edges: crate::fxhash::FxHashMap::default(),
            scheme_cache: crate::fxhash::FxHashMap::default(),
            mutual_loop_cache: crate::fxhash::FxHashMap::default(),
            rec_visited: crate::fxhash::FxHashSet::default(),
            rec_worklist: Vec::new(),
            kept_bindings: crate::fxhash::FxHashSet::default(),
            resolved_subtrees: crate::fxhash::FxHashSet::default(),
            def_schemes: crate::fxhash::FxHashMap::default(),
            param_types: crate::fxhash::FxHashMap::default(),
            solving_params: crate::fxhash::FxHashSet::default(),
            resolved: Column::new(),
            types: Column::new(),
            core: Column::new(),
        }
    }

    /// Enter a β-reduction: on success returns a guard that holds the depth bumped for its lifetime;
    /// `None` if the depth is already at [`REDUCE_DEPTH_LIMIT`] — a reduction that deep is a diverging
    /// (recursive) fold, so the evaluator DECLINES. Dropping the guard leaves the reduction.
    pub fn enter_reduction(&mut self) -> Option<ReductionGuard<'_>> {
        if self.reduce_depth >= REDUCE_DEPTH_LIMIT {
            return None;
        }
        self.reduce_depth += 1;
        Some(ReductionGuard { db: self })
    }

    /// The occurrence a previously-built constructor value was cached at, if any — the evaluator
    /// checks this before appending, so a reduction is idempotent and shared.
    pub fn cached_build(&self, key: &BuildKey) -> Option<StructId> {
        self.build_cache.get(key).copied()
    }

    /// Record that constructor `key` was built at occurrence `node`.
    pub fn cache_build(&mut self, key: BuildKey, node: StructId) {
        self.build_cache.insert(key, node);
    }

    /// The `List` occurrence that holds `id` as a child, or `None` if `id` is the root — the one step
    /// of the lexical-scope walk.
    pub fn parent_of(&self, id: StructId) -> Option<StructId> {
        *self.parent.get(id.0 as usize).unwrap_or(&None)
    }

    /// Whether the lexical-scope SKIP index covers `id` — true for a load-time node (its entry is
    /// precomputed), false for a node synthesized after load (past the index's length). An uncovered
    /// node uses the exhaustive parent walk instead. See [`scope_skip`].
    ///
    /// [`scope_skip`]: Db::scope_skip
    pub fn scope_skip_covers(&self, id: StructId) -> bool {
        (id.0 as usize) < self.scope_skip.len()
    }

    /// The nearest binding-CANDIDATE ancestor of `id` and the candidate's direct child on the path to
    /// `id` (the `from` a binder check needs), or `None` if no candidate ancestor exists. Only
    /// meaningful when [`scope_skip_covers`] is true. Lets a lexical-scope walk HOP over the non-binding
    /// spine. See [`scope_skip`].
    ///
    /// [`scope_skip`]: Db::scope_skip
    /// [`scope_skip_covers`]: Db::scope_skip_covers
    pub fn scope_skip_of(&self, id: StructId) -> Option<(StructId, StructId)> {
        self.scope_skip.get(id.0 as usize).copied().flatten()
    }

    /// `id`'s position among its parent's children (0-based). Meaningful only when `id` has a parent;
    /// a root or a node with no recorded position reads as `0`. Paired with [`parent_of`], it gives a
    /// scope walk that ascends into a sibling list (e.g. into a `let` bindings-list from one binding
    /// pair) the window bound it needs in O(1) rather than an O(k) `position()` scan of the siblings.
    pub fn child_ix_of(&self, id: StructId) -> usize {
        self.child_ix.get(id.0 as usize).copied().unwrap_or(0) as usize
    }

    /// The NAME occurrence the parameter binder `name` binds within the scope form `scope` (a `fn` or a
    /// `def`), or `None` if that scope declares no such parameter. O(1) via the load-time
    /// [`scope_binders`] index; last-wins shadowing among same-named params is baked into the index.
    /// `resolve::binder_in` uses it for a def/lambda body's parameter references (replacing an
    /// O(params)-per-reference signature scan). See [`scope_binders`].
    ///
    /// [`scope_binders`]: Db::scope_binders
    pub fn binder_in_scope(&self, scope: StructId, name: &str) -> Option<StructId> {
        self.scope_binders.get(&scope)?.get(name).copied()
    }

    /// Whether `id` is a genuine USER-PROGRAM node — one the decoded program contained, which the
    /// front-end's span table can map to a text region. A prelude binding or an evaluator-synthesized
    /// node (β-reduced body, built `(Int W)` module) is NOT one: it has no source position, so its id
    /// must never reach a diagnostic the consumer maps. The diagnostic edge filters an origin through
    /// this so a synthesized/prelude id is dropped (reported as unanchored) rather than mis-mapped.
    pub fn is_user_node(&self, id: StructId) -> bool {
        id.0 < self.user_node_count
    }

    // ── Append-during-query — the evaluator builds new nodes on demand ─────────────────────────────
    //
    // The one compile-time evaluator constructs new AST nodes as it reduces — e.g. `(Int 64)` builds a
    // width-64 module record. Those nodes are APPENDED to the arena, taking `StructId`s past the
    // load-time length; the columns are sparse and grow to reach them, so a built node is resolved /
    // typed / lowered by the ordinary demand exactly like a source node. A built node's parent is set
    // as it is created (so the lexical-scope walk works within it); built nodes are self-contained
    // (they reference only each other and existing values), so they need no enclosing scope.

    /// Append an interned leaf and an `Atom` occurrence of it, returning the occurrence's id. (Leaves
    /// are not deduplicated against the program's — a built node is small and self-contained.)
    pub fn push_atom(&mut self, leaf: Leaf) -> StructId {
        let lid = LeafId(self.ast.leaves.len() as u32);
        self.ast.leaves.push(leaf);
        let sid = StructId(self.ast.structure.len() as u32);
        self.ast.structure.push(Struct::Atom(lid));
        self.parent.push(None);
        self.child_ix.push(0);
        sid
    }

    /// Append a `List` occurrence with the given children, recording it as each child's parent so the
    /// scope walk works inside a built subtree. Returns the list's id.
    pub fn push_list(&mut self, children: Vec<StructId>) -> StructId {
        let sid = StructId(self.ast.structure.len() as u32);
        for (pos, &c) in children.iter().enumerate() {
            let ix = c.0 as usize;
            if ix < self.parent.len() {
                self.parent[ix] = Some(sid);
                self.child_ix[ix] = pos as u32;
            }
        }
        self.ast.structure.push(Struct::List(children));
        self.parent.push(None);
        self.child_ix.push(0);
        // NOTE: `scope_skip` is deliberately NOT extended for synthesized nodes — it covers only the
        // load-time arena (where the deep-nesting O(N²) lives). A synthesized β-reduced body is a
        // shallow, self-contained copy; `lookup_scope` falls back to the exhaustive parent walk for any
        // node the skip index does not cover (`Db::scope_skip_covers`), which is correct and cheap
        // there. Maintaining the skip pointer bottom-up is subtle (a node's own entry depends on its
        // not-yet-set parent), and the payoff (a shallow copy) does not warrant it.
        //
        // A SYNTHESIZED scope form (β-reduction copies a `fn`/`def` body) DOES still join the per-scope
        // binder index — `binder_in` resolves parameter references inside a reduced lambda body against
        // THIS node (the `(Int 8).wrap` regression). Cheap: only fires for a `fn`/`def` head.
        self.index_scope_binders(sid);
        sid
    }

    /// If `form` is a `fn`/`def` scope form, (re)build its entry in the per-scope binder index — the
    /// incremental counterpart of the load-time [`build_scope_binders`], for a node synthesized after
    /// load. Idempotent: re-indexing the same form recomputes the same last-wins map.
    fn index_scope_binders(&mut self, form: StructId) {
        let params_occ = if let Some(tail) = self.ast.as_form(form, "fn") {
            tail.first().copied()
        } else if let Some(tail) = self.ast.as_form(form, "def") {
            tail.first().copied()
        } else {
            return;
        };
        let Some(params_occ) = params_occ else {
            return;
        };
        let Struct::List(children) = self.ast.get(params_occ) else {
            return;
        };
        let is_def = self.ast.as_form(form, "def").is_some();
        let params: Vec<StructId> = if is_def {
            children.iter().skip(1).copied().collect()
        } else {
            children.clone()
        };
        let mut map: crate::fxhash::FxHashMap<String, StructId> =
            crate::fxhash::FxHashMap::default();
        for p in params {
            // A parameter is a bare name atom or an annotated binder `(: name T)` (its first child is
            // the name) — the two shapes `build_scope_binders`/`param_name` recognize.
            let binder = if let Some(n) = self.ast.as_name(p) {
                Some((n.to_string(), p))
            } else if let Some(tail) = self.ast.as_form(p, ":") {
                tail.first()
                    .and_then(|&no| self.ast.as_name(no).map(|n| (n.to_string(), no)))
            } else {
                None
            };
            if let Some((n, occ)) = binder {
                map.insert(n, occ); // last-wins
            }
        }
        if !map.is_empty() {
            self.scope_binders.insert(form, map);
        }
    }

    /// Convenience: append an `Atom` of a `Name`.
    pub fn push_name(&mut self, name: &str) -> StructId {
        self.push_atom(Leaf::Name(name.to_string()))
    }

    /// Append an `Atom` of a STRING literal. A synthesized primitive-constructor head (`"tuple"`/
    /// `"record"`) is a string, not a name — unspellable as an identifier, so unshadowable.
    pub fn push_str(&mut self, s: &str) -> StructId {
        self.push_atom(Leaf::Str(s.to_string()))
    }

    /// The definition of the given name, if one exists — how an export resolves its target and how a
    /// later call resolves its callee (by name against the index, reading a signature, not a body).
    /// O(1) via the `def_name_index` map (built at load). `resolve_name` calls this for every non-local
    /// name reference, so a linear scan here made resolution O(N²) on a many-def program.
    pub fn def_by_name(&self, name: &str) -> Option<usize> {
        self.def_name_index.get(name).copied()
    }

    /// The index in [`defs`] of the def whose body is the occurrence `body`, if any — an O(1) reverse
    /// lookup of `defs[i].body`, built at load. This is the hot query the recursion analysis makes of
    /// every callee edge and lowering/inference make per call; it replaces the equivalent linear
    /// `defs.iter().position(|d| d.body == Some(body))` scan.
    ///
    /// [`defs`]: Db::defs
    pub fn def_index_by_body(&self, body: StructId) -> Option<usize> {
        self.def_by_body.get(&body).copied()
    }

    /// The synthesized SUM RECORD of the type named `name`, if one is declared — how a `(type NAME …)`
    /// name resolves, exactly parallel to `def_by_name`. Returns the record occurrence a bare `NAME`
    /// denotes (a `Ref` to it), so `Option`, `Option.Some`, and `(: x Option)` take the ordinary
    /// member-access / `(meta t)` paths. First-wins on the name (like `def_by_name`); the record's own
    /// `(meta t)` occurrence — NOT the name — is the type's IDENTITY, so two same-named declarations are
    /// still distinct types (unlike a name-keyed map, which would clobber). A duplicate type NAME in one
    /// scope is a well-formedness concern for the checker, not this index.
    pub fn type_decl_by_name(&self, name: &str) -> Option<StructId> {
        self.type_decls
            .iter()
            .find(|t| t.name == name)
            .and_then(|t| t.synth)
    }

    /// The `(index, TypeDecl)` of the sum whose DECLARATION OCCURRENCE is `occ` — the reverse of the
    /// nominal identity a `Ty::Sum { decl }` carries. Used by the escape renderer to recover a sum's
    /// variant names + payload types from its type-value. `None` if `occ` names no declaration.
    pub fn type_decl_by_occ(&self, occ: StructId) -> Option<&TypeDecl> {
        self.type_decls.iter().find(|t| t.occ == occ)
    }

    /// The top-level items whose head is a form the compiler does NOT model — `(effect …)`, `(pragma
    /// …)`, or any other unrecognized top-level declaration — as `(head-name, occurrence)` pairs. A
    /// program containing one DECLINES (decline-don't-miscompile): the compiler cannot claim to compile a
    /// program whose top level it does not fully understand, so it declines rather than silently ignoring
    /// the unknown item and compiling the rest (which would run `main` as if the declaration were absent
    /// — e.g. an `(effect E …)` with a duplicate operation would go unchecked). A bare expression at the
    /// root (a `(do …)`/module item that is an application/name/literal, not a declaration form) is NOT
    /// flagged — only a LIST whose head is an unrecognized NAME (a declaration-shaped form) is.
    pub fn unknown_top_forms(&self) -> Vec<(String, StructId)> {
        let mut out = Vec::new();
        for item in top_items(&self.ast) {
            // Flag a `(head …)` list whose NAME head is neither a recognized TOP-LEVEL declaration
            // (`type` — `def`/`export`/`module`/`do` are grammar) NOR a grammar/expression head. So an
            // unmodeled top-level DECLARATION like `(effect …)`/`(pragma …)` declines, while a top-level
            // EXPRESSION (a `do`-item that is an `if`/application/`match`/… — grammar or an application
            // whose head is a bound value) is left to the ordinary fold. A bare atom root is fine.
            if let Some(head) = self.ast.head_name(item)
                && !TOP_LEVEL_FORMS.contains(&head)
                && !crate::resolve::is_grammar_head(head)
                // An APPLICATION `(f x)` at the top level (a `do`-item calling a bound function) has a
                // head that is a bound name, not an unknown declaration — do not flag it. Only a head
                // that resolves to NOTHING (an unbound name used as a declaration keyword) is unmodeled.
                && self.def_by_name(head).is_none()
                && !self.prelude.contains_key(head)
                && self.type_decl_by_name(head).is_none()
            {
                out.push((head.to_string(), item));
            }
        }
        out
    }
}

/// Build the parent index AND the child-position index in one pass: for each structure occurrence,
/// the `List` occurrence that holds it as a child (`None` for the root) and its position among that
/// parent's children (`0` when it has no parent). One pass over the whole arena — deterministic (a
/// child's parent and position are a fixed function of the arena, no ordering or address involved).
fn parent_index(ast: &Arenas) -> (Vec<Option<StructId>>, Vec<u32>) {
    let mut parent = vec![None; ast.structure.len()];
    let mut child_ix = vec![0u32; ast.structure.len()];
    for i in 0..ast.structure.len() {
        if let Struct::List(children) = &ast.structure[i] {
            for (pos, &child) in children.iter().enumerate() {
                parent[child.0 as usize] = Some(StructId(i as u32));
                child_ix[child.0 as usize] = pos as u32;
            }
        }
    }
    (parent, child_ix)
}

/// Whether `form` is a binding CANDIDATE — a form `resolve::binder_in` could bind a name in, for SOME
/// child ascended from. This is the structural (name-agnostic, child-agnostic) OVER-approximation the
/// scope-skip index hops between: a `let`/`fn`/`def` (by head), a `let` BINDINGS-LIST (its parent is a
/// `let` and it is that let's first tail element), or a MATCH ARM (its parent is a `match` and it is a
/// non-scrutinee tail element). It is SOUND to over-approximate (a candidate that turns out not to bind
/// THIS name/child just returns `None` from `binder_in`, and the walk continues), but it must never
/// UNDER-approximate (skipping a real binder would mis-resolve). Kept in lockstep with `binder_in`'s
/// cases — a new binding form must be added here too. `parent` is the precomputed parent index.
fn is_binding_candidate(ast: &Arenas, parent: &[Option<StructId>], form: StructId) -> bool {
    // By head: a `let`/`fn`/`def` form, or a match-arm `(guard <binder> <cond>)` — the guard binds the
    // pattern's binder in scope for the guard COND (`binder_in`'s Case 5g / `guard_cond_binds`). Without
    // the guard as a candidate the scope-skip index would hop PAST it and Case 5g would never fire, so a
    // guard reference to its binder would be spuriously unbound.
    if let Some(h) = ast.head_name(form)
        && matches!(h, "let" | "fn" | "def" | "guard")
    {
        return true;
    }
    // By parent shape: a `let` bindings-list (parent is a `let`, `form` its first tail element), or a
    // `match` arm (parent is a `match`, `form` a tail element after the scrutinee at tail position 0).
    let Some(p) = parent.get(form.0 as usize).copied().flatten() else {
        return false;
    };
    if let Some(tail) = ast.as_form(p, "let")
        && tail.first().copied() == Some(form)
    {
        return true; // the bindings-list
    }
    if let Some(tail) = ast.as_form(p, "match") {
        // tail = [scrutinee, arm…]; `form` is an arm iff it is a tail element other than the scrutinee.
        if tail.first().copied() != Some(form) && tail.contains(&form) {
            return true;
        }
    }
    false
}

/// Build the per-node lexical-scope SKIP index (`Db::scope_skip`): for each node, the nearest STRICT
/// ANCESTOR that is a binding candidate ([`is_binding_candidate`]) plus that candidate's direct child
/// on the path to this node (the `from` the resolver's `binder_in` needs). `lookup_scope` follows these
/// to hop over the non-binding spine, so a reference costs O(enclosing binders), not O(nesting depth).
///
/// Path-compression, no id-ordering assumption: for a node whose parent is `P`, its entry is `(P, node)`
/// if `P` is a candidate, else the SAME entry as `P` (same candidate, same entry child — the candidate
/// above `P` is reached from `node` through the very child `P` reaches it through). Computed with an
/// explicit stack per node, memoizing as it unwinds, so each node is resolved once (amortized O(n)).
fn build_scope_skip(
    ast: &Arenas,
    parent: &[Option<StructId>],
) -> Vec<Option<(StructId, StructId)>> {
    let n = ast.structure.len();
    let mut skip: Vec<Option<(StructId, StructId)>> = vec![None; n];
    let mut done = vec![false; n];
    // A reusable stack of nodes whose entry is not yet computed, pushed child→ancestor until a resolved
    // (or root) node, then filled on the way back down.
    let mut stack: Vec<StructId> = Vec::new();
    for start in 0..n {
        if done[start] {
            continue;
        }
        stack.clear();
        let mut cur = StructId(start as u32);
        // Descend the parent chain collecting unresolved nodes until we hit a resolved node or the root.
        loop {
            if done[cur.0 as usize] {
                break;
            }
            done[cur.0 as usize] = true; // mark now; its `skip` is filled below (before any read of it)
            stack.push(cur);
            match parent[cur.0 as usize] {
                Some(p) => cur = p,
                None => break, // root: no ancestor, entry stays None
            }
        }
        // Unwind: fill each node's skip from its parent's (already-final) skip.
        while let Some(node) = stack.pop() {
            let entry = match parent[node.0 as usize] {
                Some(p) => {
                    if is_binding_candidate(ast, parent, p) {
                        Some((p, node)) // parent is the nearest candidate; enter it through `node`
                    } else {
                        skip[p.0 as usize] // inherit parent's candidate + its entry child
                    }
                }
                None => None,
            };
            skip[node.0 as usize] = entry;
        }
    }
    skip
}

/// Build the per-scope-form parameter binder index (`Db::scope_binders`): for every `fn` and `def`
/// occurrence in the arena, a `name → binder-name-occurrence` map of its parameters, LAST-wins (a
/// later param of the same name overwrites the earlier — matching the old forward scan's shadowing).
/// One pass over the arena. This replaces the O(params)-per-reference signature scan `binder_in`'s
/// Case-3/Case-4 did (O(N²) for N references into an N-param scope) with an O(1) probe.
///
/// A parameter is either a bare name atom (`a`) or an annotated binder `(: a T)` whose FIRST child is
/// the name — the same two shapes `resolve::param_name` recognizes. The scope FORM's occurrence is the
/// key, so `binder_in` (which has the form id in hand) can probe directly.
fn build_scope_binders(
    ast: &Arenas,
) -> crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashMap<String, StructId>> {
    // The name a parameter occurrence declares, with its NAME occurrence — bare `a` or `(: a T)`.
    // Mirrors `resolve::param_name` (kept in sync; a divergence would resolve a param wrong).
    fn param_binder(ast: &Arenas, param: StructId) -> Option<(&str, StructId)> {
        if let Some(n) = ast.as_name(param) {
            return Some((n, param));
        }
        let tail = ast.as_form(param, ":")?;
        let name_occ = *tail.first()?;
        let n = ast.as_name(name_occ)?;
        Some((n, name_occ))
    }

    let mut out: crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashMap<String, StructId>> =
        crate::fxhash::FxHashMap::default();
    for i in 0..ast.structure.len() {
        let form = StructId(i as u32);
        // A `(fn (params…) body)` — the parameters are the children of its first tail element.
        // A `(def (NAME param…) body)` — the parameters are the signature's children after NAME.
        let params_occ = if let Some(tail) = ast.as_form(form, "fn") {
            tail.first().copied()
        } else if let Some(tail) = ast.as_form(form, "def") {
            tail.first().copied()
        } else {
            None
        };
        let Some(params_occ) = params_occ else {
            continue;
        };
        let Struct::List(children) = ast.get(params_occ) else {
            continue;
        };
        // For a `fn` the whole list is parameters; for a `def` the first child is the def NAME, not a
        // parameter — skip it. Distinguish by which form matched (a `def`'s signature holds the name).
        let is_def = ast.as_form(form, "def").is_some();
        let params = if is_def {
            &children[1.min(children.len())..]
        } else {
            &children[..]
        };
        if params.is_empty() {
            continue;
        }
        let mut map: crate::fxhash::FxHashMap<String, StructId> =
            crate::fxhash::FxHashMap::default();
        for &p in params {
            if let Some((n, name_occ)) = param_binder(ast, p) {
                map.insert(n.to_string(), name_occ); // last-wins
            }
        }
        if !map.is_empty() {
            out.insert(form, map);
        }
    }
    out
}

/// The one cheap top-level scan: gather the definitions, export requests, and `(type …)` declarations
/// from the top form only, without entering any body. Recognizes `(module NAME item…)`, a bare
/// `(do item…)`, or a lone item.
fn scan_top_level(ast: &Arenas) -> (Vec<Def>, Vec<Export>, Vec<TypeDecl>) {
    let mut defs: Vec<Def> = Vec::new();
    let mut exports: Vec<Export> = Vec::new();
    let mut types: Vec<TypeDecl> = Vec::new();

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
            defs.push(Def {
                name,
                sig_occ,
                params,
                body,
            });
        } else if let Some(tail) = ast.as_form(item, "export")
            && let Some(name) = tail.first().and_then(|&s| ast.as_name(s))
        {
            exports.push(Export {
                name: name.to_string(),
                def: None,
                occ: item,
            });
        } else if ast.as_form(item, "type").is_some()
            && let Some(decl) = scan_type_decl(ast, item)
        {
            types.push(decl);
        }
    }

    // Resolve each export's target index by name against the gathered defs (a signature read, not a
    // body read).
    // Resolve each export to the def it names. A per-export `defs.iter().position(|d| d.name == …)`
    // is an O(defs) STRING-comparison scan, so N exports over N defs is O(N²) memcmp (the dominant
    // cost of loading a many-export program). Build a `name → first def index` map once, then each
    // export resolves in O(1). First-wins matches `position`'s first-match (a duplicate def name keeps
    // the earlier def, which the well-formedness pass reports separately).
    let mut def_of_name: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, d) in defs.iter().enumerate() {
        def_of_name.entry(d.name.as_str()).or_insert(i);
    }
    for e in &mut exports {
        e.def = def_of_name.get(e.name.as_str()).copied();
    }

    (defs, exports, types)
}

/// Scan a `(type NAME variant…)` declaration at `item` into a [`TypeDecl`] — the name, each variant
/// (its name occurrence + payload type occurrences), and the implicit type parameters (free lowercase
/// payload names, first-appearance order). `synth` is left `None` (filled by `sums::synthesize`).
/// Returns `None` if `item` is not a `(type …)` form. Shared by the top-level scan and the built-in
/// prelude-sum synthesis, so a prelude `Option`/`Result` declaration scans exactly like a user one.
pub(crate) fn scan_type_decl(ast: &Arenas, item: StructId) -> Option<TypeDecl> {
    let tail = ast.as_form(item, "type")?;
    let name = tail
        .first()
        .and_then(|&s| ast.as_name(s))
        .unwrap_or("")
        .to_string();
    let mut variants = Vec::new();
    for &v in tail.iter().skip(1) {
        let (name_occ, payloads) = match ast.get(v) {
            // A bare nullary variant name — no payloads.
            Struct::Atom(_) => (v, Vec::new()),
            // `(vname payload…)` — the variant name is the list head; the rest are payload type
            // occurrences in declaration order.
            Struct::List(children) => match children.first() {
                Some(&head) => (head, children.iter().skip(1).copied().collect()),
                None => continue,
            },
        };
        if let Some(vname) = ast.as_name(name_occ) {
            variants.push(Variant {
                name: vname.to_string(),
                name_occ,
                payloads,
            });
        }
    }
    // Collect the IMPLICIT type parameters: a free LOWERCASE name in any payload, in first-appearance
    // order across the variants (`(type Option (Some a) None)` → `["a"]`). A Capitalized name is a type.
    let mut params: Vec<String> = Vec::new();
    for variant in &variants {
        for &p in &variant.payloads {
            collect_type_params(ast, p, &mut params);
        }
    }
    Some(TypeDecl {
        name,
        occ: item,
        params,
        variants,
        synth: None,
    })
}

/// Collect the IMPLICIT type parameters mentioned in a payload type expression at `occ`, appending each
/// new one to `params` in first-appearance order. A parameter is a free LOWERCASE name (types are
/// Capitalized — `Int64`, `Bool`, `Option` — so a lowercase name in type position is a type variable,
/// the same convention the prelude's operator type-lambdas use with `a`). Descends a type application
/// `(Option a)` / `(Tuple a b)` into its arguments. A duplicate is not re-added (a `HashSet`-like
/// linear check keeps the small param list ordered by first appearance).
fn collect_type_params(ast: &Arenas, occ: StructId, params: &mut Vec<String>) {
    match ast.get(occ) {
        Struct::Atom(_) => {
            if let Some(n) = ast.as_name(occ)
                && n.starts_with(|c: char| c.is_ascii_lowercase())
                && !params.iter().any(|p| p == n)
            {
                params.push(n.to_string());
            }
        }
        // A type application `(Head arg…)` — descend into every child (the head of a nested application
        // may itself be a name, but a Capitalized head is a type constructor, not a parameter; the
        // lowercase filter above handles that). This reaches a parameter nested in `(Option a)`.
        Struct::List(children) => {
            for &c in children {
                collect_type_params(ast, c, params);
            }
        }
    }
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

/// The recognized TOP-LEVEL form heads — the constructs the scan turns into an index entry. A top-level
/// item whose head is NOT one of these is a construct the compiler does not model (e.g. `(effect …)`,
/// `(pragma …)`), and the whole program DECLINES rather than silently ignoring it and compiling the rest
/// (decline-don't-miscompile — a program with an unmodeled declaration is out of scope, not partially
/// meaningful). Kept in sync with `scan_top_level`'s branches.
const TOP_LEVEL_FORMS: &[&str] = &["def", "export", "type"];

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
