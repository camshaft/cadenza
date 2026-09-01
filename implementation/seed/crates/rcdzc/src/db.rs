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
//!
//! Because name resolution (`resolve`), type checking (`infer`), and lowering (`lower`) each fill a
//! COLUMN over the IR, every such transformation is applied to the intermediate representation, never
//! as a side effect of emitting instruction bytes — the backend runs only after these columns are
//! filled and reads them, so a transformation is a fact about the IR rather than something that happens
//! during serialization.
//= spec/capabilities/compiler-pipeline.md#emission-serializes-a-lowered-representation
//# The compiler MUST perform name resolution, type checking, and each transformation it applies to a program as a transformation of its intermediate representation rather than as an effect of emitting instruction bytes.

use crate::arena::Column;
use crate::ast::{Arenas, CompoundCtor, Leaf, LeafId, Struct, StructId};
use crate::core::Core;
use crate::resolved::Resolved;
use crate::ty::Ty;
use std::collections::BTreeMap;

#[cfg(test)]
thread_local! {
    /// Test-only: total node-visits by [`Db::collect_reduced_callables`] since the last reset. A
    /// deeply-nested inlining re-walked its growing reduced term per β-reduction = O(N²); the
    /// `reduced_callable_walked` visited set makes it O(N). This counter is the noise-free regression signal
    /// (a wall-clock ratio is diluted by the rest of `check`) — see the lock-in test.
    pub(crate) static COLLECT_REDUCED_CALLABLES_VISITS: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total `lower::build_tree_ft` INVOCATIONS since the last reset — the decision-tree
    /// compiler's recursion count, a pure function of the program. A match whose arms each test ≥2 literal
    /// columns (`(tuple 0 0 a)` — a transition-table dispatch) compiled the shared fall-through matrix in
    /// BOTH a column-test's `then_` and its `els` at every column, so the recursion was O(2^arms) (a 20-arm
    /// 2-column match: ~5s to check). Compiling that fall-through ONCE into a shared `Rc<SumCont>` and
    /// threading it (for a non-refining Int/Str probe) makes the recursion O(arms). This counter is the
    /// noise-free regression signal (a wall-clock ratio is diluted by the rest of `check`) — see
    /// `a_multi_column_literal_match_compiles_in_linear_time`. (Also guards the fix-64 wide single-column
    /// case: an N-arm single-column match is O(N) recursions, not O(N²).)
    pub(crate) static BUILD_TREE_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Test-only: total node-visits by `effects::check_no_home_walk` (the CDZ0401 no-home check) since the
    /// last reset. Following a callee body had NO dedup, so a helper called from N sites re-walked its
    /// O(N)-body once per site = O(N²); the `(callee, handled-set)` follow-dedup makes it O(N). The
    /// noise-free regression signal (a wall-clock ratio is diluted by the rest of `check`) — see the
    /// `check_no_home_follows_a_shared_callee_body_once_per_handler_context` lock-in test.
    pub(crate) static CHECK_NO_HOME_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Test-only: total record-field KEYS enumerated while building `record_field_index` entries since the
    /// last reset. `eval::runtime_member_index` found a field's sorted slot by a LINEAR `keys().position()`
    /// scan — O(fields) PER projection, so a wide record projected field-by-field was O(N²). The per-type
    /// index map (built once, keyed by the type `Rc`) makes the total keys enumerated O(fields) regardless
    /// of projection count. This counter is the noise-free regression signal (a wall-clock ratio is diluted
    /// by inference's own cost) — see the `runtime_record_field_projection_indexes_in_bounded_time` test.
    pub(crate) static RECORD_FIELD_INDEX_KEYS_SCANNED: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total compound-payload elements walked by `infer::ty_has_free_var` since the last reset —
    /// the fields of a `Ty::Record`/elements of a `Ty::Tuple` it actually enumerates (a cache MISS walks
    /// them; a HIT is O(1)). The `type_of` memoization guard (`!has_free_var`) ran the full O(N) walk of a
    /// wide record type once PER node referencing it → O(N²); the `Db::ty_has_free_var` per-`Rc` cache makes
    /// the total elements walked O(N) regardless of reference count. The noise-free regression signal (a
    /// wall-clock ratio is diluted by inference's own cost) — see
    /// `ty_has_free_var_walks_a_shared_record_type_once`.
    pub(crate) static TY_HAS_FREE_VAR_ELEMS_WALKED: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total `Ty` nodes VISITED by `subst_template_vars` (the generic-nominal template
    /// substitution) since the last reset. `normalize_sum` builds `Ty::Nominal { inner }` by substituting
    /// the type args into the template; before `Nominal.inner` was an `Rc`, a NESTED nominal `(Box (Box …
    /// Int64))` deep-CLONED the child into `inner` at every level, so the substituted `Ty` — and the work
    /// to build+re-substitute it — DOUBLED per level = O(2^depth). With `inner: Rc<Ty>` the child is SHARED,
    /// so the substitution work is O(depth). The noise-free regression signal — see
    /// `a_deeply_nested_generic_nominal_annotation_reduces_to_a_linear_size_type`.
    pub(crate) static SUBST_TEMPLATE_VARS_VISITS: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total `resolve::binder_in` calls (one per enclosing form a name-resolution's scope walk
    /// visits) since the last reset. The runtime-map-match desugar builds an O(arms)-DEEP synthesized
    /// `(if <k-present> body <else>)` presence chain; without a scope-skip entry for those synth nodes, a
    /// prelude name (`Map`/`Some`/`None`) in an inner arm walked O(depth) enclosing forms → O(arms²) total.
    /// `extend_scope_skip_pass_through` makes each such resolution hop O(1). This counter is the noise-free
    /// signal (a wall-clock ratio is diluted by the rest of check) — see
    /// `a_wide_runtime_map_match_resolves_synth_names_in_bounded_time`.
    pub(crate) static BINDER_IN_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Test-only: total times `resolve::binder_in` ENTERED the match-arm case cascade (Cases 5..6mg) since
    /// the last reset — i.e. hops NOT fast-rejected by the per-arm BINDER-name over-approx
    /// (`Db::arm_cannot_bind`). A reference to a global/prelude/CTOR name in a DEEPLY-NESTED match ascends
    /// O(depth) arms; because the fast-reject set excludes pattern HEADS, a ctor name (`Some`/`None`) is
    /// fast-rejected at every arm → cascade entries stay O(N) (only genuine binder-name references enter,
    /// at their binding arm). An all-atoms set (or no fast-reject) would keep ctor-name refs entering the
    /// cascade at every enclosing arm → O(N²). `BINDER_IN_CALLS` cannot pin this (it counts INVOCATIONS,
    /// unchanged — the fast-reject is inside binder_in); only cascade ENTRIES drop. Noise-free signal — see
    /// `arm_cascade_entries_stay_linear_on_a_nested_match`.
    pub(crate) static ARM_CASCADE_ENTRIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Test-only: total LEADING elements enumerated by `resolve::find_leading_binder_in_list_pattern`
    /// since the last reset — the elements examined while building a list pattern's binder index PLUS any
    /// visited by the linear fallback (a non-simple pattern). Resolving a name against a `(list p… .. r)`
    /// arm re-scanned the pattern's leading positions from 0 on EVERY reference (positive to the binder's
    /// position, negative — a prelude/outer name the pattern does not bind — over the whole pattern), so a
    /// wide `(list a0 … aN .. r)` destructure (or the synth `(list __le0 … __leN .. r)` the refutable-
    /// literal-element desugar builds) referenced from N sites was O(N²). The per-pattern binder index
    /// (`Db::simple_list_binders`, built once per pattern) makes each lookup O(1), so the total elements
    /// enumerated is O(N) regardless of reference count. Noise-free regression signal — see
    /// `a_wide_list_pattern_resolves_element_binders_in_bounded_time`.
    pub(crate) static LIST_PATTERN_BINDER_ELEMS_SCANNED: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total nodes VISITED by `lower::collect_binding_uses` since the last reset. `lower_let`
    /// collects each binding's use facts by walking its whole `let` REGION (all inits + body); for a DEEP
    /// nested `let` chain `(let ((v0 …)) (let ((v1 …)) … body))` each of the N levels' `lower_let` re-walked
    /// its body — the entire deeper O(N−k) chain — so Σ = O(N²) (the deep-nested twin of the fix-44 wide
    /// case). The `Db::binding_uses_cache` per-node memo makes each subtree contribute its facts ONCE, so
    /// the total nodes visited is O(N). Noise-free regression signal — see
    /// `a_deep_nested_let_chain_collects_binding_uses_in_bounded_time`.
    pub(crate) static COLLECT_BINDING_USES_VISITS: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: the MAX field entries a SINGLE `eval::project_meta` reverse-scan touched since the last
    /// reset (a running max, not a total). `project_meta` reads a `meta`-namespace field off a record; field
    /// keys sort `(namespace, name)` with a USER field (`None`) BELOW `Some("meta")`, so the meta fields are
    /// a contiguous block at the TOP and a REVERSE scan reaches them without touching the O(width) user
    /// fields (it breaks on descending below the meta block). A FORWARD scan (or any walk that doesn't stop)
    /// would be O(record-width) per call, which — called per node on a WIDE record — is the O(N²)
    /// `203f8588` fixed. This counter locks the PER-CALL scan depth to O(meta-fields), NOT O(width): the max
    /// stays bounded (~3) regardless of how wide the record is. See
    /// `project_meta_reads_a_meta_field_without_scanning_the_wide_user_block`.
    pub(crate) static PROJECT_META_FIELDS_VISITED: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total UN-CACHED `effects::subtree_performs` computations (the `_uncached` body) since
    /// the last reset — i.e. the actual per-node perform-verdict work, NOT the cache hits. The frame-free
    /// effect fold's `strongly_pure`/`pure_hole` classifiers call `subtree_performs` at MANY nodes as they
    /// descend a handle body, and `strongly_pure` re-ran the WHOLE-subtree walk at every node, so a wide/
    /// deep body recomputed the same node's verdict O(size) times (the scan O(size²), the fold super-linear).
    /// Memoizing per `(node, ctx.key)` collapses the repeats — so this count grows O(size), not O(size²).
    /// The noise-free regression signal (a wall-clock ratio false-fails under fleet load) — see
    /// `the_effect_fold_pure_classifier_scales_linearly_over_a_wide_context`.
    pub(crate) static SUBTREE_PERFORMS_UNCACHED_CALLS: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total `variant_closest_matches` MISSES since the last reset — the far-miss pattern-head
    /// "closest matches" LIST actually COMPUTED (`suggest::closest_matches` sorts all N variants by edit
    /// distance, O(N log N)), NOT the memo hits. A wrong-sum ctor arm (always a far miss) matched from N
    /// sites against a wide sum re-ran the sort each → O(N² log N); the per-`(decl, key)` memo collapses
    /// repeats, so a program's misses = its DISTINCT (decl, key) wrong heads, independent of the site count.
    /// The noise-free regression signal (a wall-clock ratio false-fails under fleet load) — see
    /// `many_wrong_sum_ctor_arms_list_the_matched_variants_in_bounded_time`.
    pub(crate) static VARIANT_CLOSEST_MATCHES_MISSES: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total `no_field_suggestion` MISSES since the last reset — the record-field "did you
    /// mean?" (winner, hint) pair actually COMPUTED (`suggest::nearest` + `did_you_mean` each build the
    /// record's O(fields) name list and edit-distance-scan it), NOT the memo hits. A wide record (N
    /// fields) with a typo'd field accessed from N sites re-ran that per access → O(N²); the per-
    /// `(reduced-record occ, key)` memo collapses repeats, so a program's misses = its DISTINCT
    /// (record, key) typo'd accesses, independent of the site count. The noise-free regression signal
    /// (a wall-clock ratio false-fails under fleet load) — see
    /// `many_typod_field_accesses_of_one_wide_record_suggest_in_bounded_time`.
    pub(crate) static NO_FIELD_SUGGESTION_MISSES: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total nodes the `quote::reify_quotes` POSITION passes enumerated since the last reset —
    /// the O(N) work (`pattern_position_nodes`' parent-map build + downward walk, plus the reverse
    /// per-node plan loop's shape probes) that runs ONLY when the program actually contains a
    /// `quote`/`quasiquote` FORM. `reify_quotes` runs at EVERY load, but a quote-FREE program (the
    /// overwhelming common case) fast-bails on a single O(leaves) name scan before touching any of it, so
    /// this counter stays 0 for such a program and grows O(N) only for a genuine quote program. The
    /// noise-free regression signal that the fast-bail holds — see
    /// `a_quote_free_program_skips_the_reify_quotes_position_scan`.
    pub(crate) static REIFY_QUOTES_POSITION_SCAN_NODES: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total nodes the `eval_ast::desugar_eval` per-node scan enumerated since the last reset —
    /// the O(N) `as_form(id,"eval")` probe over every node, which runs ONLY when the program actually
    /// contains an `(eval …)` form. `desugar_eval` runs at EVERY load but a program with no `(eval …)`
    /// (the overwhelming common case) fast-bails on a single O(leaves) name prescan before the scan, so
    /// this counter stays 0 for such a program and grows O(N) only for a genuine eval program. See
    /// `an_eval_free_program_skips_the_desugar_eval_node_scan`.
    pub(crate) static DESUGAR_EVAL_SCAN_NODES: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total nodes the `tagged_template::expand` per-node scan enumerated since the last reset
    /// — the O(N) `rewrite_of` probe over every node, which runs ONLY when the program actually contains
    /// a `(tagged-template …)` form (the reader emits one for the ML surface `tag"…{expr}…"`). `expand`
    /// runs at EVERY load but a program with no tagged template (the common case) fast-bails on a single
    /// O(leaves) name prescan, so this counter stays 0 for such a program. See
    /// `a_tagged_template_free_program_skips_the_expand_node_scan`.
    pub(crate) static TAGGED_TEMPLATE_SCAN_NODES: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };

    /// Test-only: total nodes the `infer::check_unknown_units` per-node scan enumerated since the last
    /// reset — the O(N) `(Unit.of …)`-head probe (`resolved_ref` + `meta_apply_of`) over the program.
    /// The pass runs at EVERY compile/check, but a genuine unknown-unit fault can only ever anchor at a
    /// USER node (a prelude / evaluator-synthesized node has no source span, so a fault reported there is
    /// nulled by `sanitize_origin`; a β-copy relocates to its user origin, which the scan already covers).
    /// So the scan is bounded to `user_node_count` — it never touches the O(prelude) built-in nodes that
    /// a full-structure walk would. This counter pins that bound: ONE `check_unknown_units` invocation adds
    /// exactly `user_node_count` (never the full structure length). It is CUMULATIVE-since-reset (like the
    /// sibling scan counters), so a test that reads it MUST reset it to 0 immediately before the single
    /// `diagnostics()`/compile it measures — repeated compiles on the same thread without a reset legitimately
    /// sum past `user_node_count` (the per-`Db`/thread_local metric-contamination discipline). See
    /// `check_unknown_units_scans_only_user_nodes_not_the_prelude`.
    pub(crate) static CHECK_UNKNOWN_UNITS_SCAN_NODES: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

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
    /// INTERNAL def — a MODULE MEMBER or DO-LOCAL function registered so a RECURSIVE call to it can lower
    /// to a `Core::Call` (a standalone emittable wasm function; `crate::modules::register_callable`). It
    /// is NOT in `def_name_index` (its NAME resolves by lexical scope — Case R / `do_local_binds` — per
    /// the no-keys-outside-the-prelude rule) and is NOT a program EXPORT candidate or unused-def-warning
    /// target; it exists purely to give a recursive nested function a stable `db.defs` index. Its body is
    /// the member's ORIGINAL body, its params the original signature params, so `def_by_body`/`def_scheme`
    /// find it and the body's param references resolve to the same occurrences (via the synth `(fn …)`).
    /// `false` for every ordinary top-level / accum / effect-specialization def.
    pub internal: bool,
}

/// One CONCRETE INSTANTIATION of a generic / ad-hoc-polymorphic definition — a record monomorphization
/// appends every time it synthesizes a specialized copy (`crate::lower::type_specialize`), so the
/// compiler can ENUMERATE the concrete instances of a source definition after a lower pass (the
/// `Instantiations` sidecar query, `cdz instantiations`). The `type_specializations` memo already keys a
/// specialization by `(orig-body, instantiation-key)`, but that key renders a `const`-dictionary
/// argument as an opaque fingerprint; this record instead carries a HUMAN-READABLE per-argument
/// description, so a report can SHOW which concrete dictionary an ad-hoc-polymorphic call baked in — the
/// distinguishing data of an instantiation. One record per DISTINCT specialization (appended at the memo
/// MISS, alongside the memo insert), so it never double-counts a call site that reuses an instance.
#[derive(Clone, PartialEq, Debug)]
pub struct Instantiation {
    /// The GENERIC definition's body occurrence — the identity `type_specialize` keyed the copy on, which
    /// `def_index_by_body` maps back to the source definition (so a query by NAME finds its instances).
    pub orig_body: StructId,
    /// The synthesized specialization's `db.defs` index (its emitted monomorphic function).
    pub spec_index: usize,
    /// The concrete instantiation, one entry per parameter in signature order: a kept RUNTIME parameter
    /// as `name: TYPE`, an ERASED compile-time parameter (a type-valued or `const` argument) as
    /// `const name = VALUE` — the type-value for a type argument, the inlined SOURCE text for a `const`
    /// dictionary/constant.
    pub args: Vec<String>,
}

/// The file-scoped name-resolution table for a multi-file PACKAGE (`DESIGN-package-linking.md` §4),
/// derived once at load from the [`crate::link::Linkage`]. It answers the two file-scoped questions
/// `resolve_name` asks when a package is linked:
///  - which FILE a reference's node belongs to (`file_of` — a `StructId` range lookup over the demux
///    table, with a `None` fallback for a synthesized / β-copied / prelude node);
///  - what a bare NAME resolves to WITHIN a given file — its own top-level def, or an imported name
///    (mapped to the exporting file's def), or nothing (fall through to prelude / unbound).
pub(crate) struct FileScopeTable {
    /// The `StructId → file` demux ranges, splice order.
    files: Vec<crate::link::FileSpan>,
    /// Per file, the visible top-level VALUE names → their index in [`Db::defs`]. Own defs plus the
    /// file's imports (an import maps the local name to the exporting file's def). A name absent here
    /// is not a package-level def in that file — resolution falls through to the prelude, then unbound.
    visible: Vec<crate::fxhash::FxHashMap<String, usize>>,
    /// Per file, the visible TYPE names → the synthesized record occurrence the type name resolves to
    /// (`TypeDecl::synth`). Own `(type …)` declarations plus the types the file imports. A type name
    /// absent here is NOT visible in that file — a sibling file's type is invisible unless imported
    /// (`DESIGN-package-linking.md` §8.3 residual (a), closed here). This is the type-name analogue of
    /// `visible`, and is what makes cross-file type privacy real: without it, `type_decl_by_name`'s flat
    /// global index let any file name any sibling's type + construct its variants with no import.
    visible_types: Vec<crate::fxhash::FxHashMap<String, StructId>>,
    /// Per file, the visible bare VARIANT-CONSTRUCTOR names → the ctor record occurrence
    /// (`Variant::ctor`). Populated from `visible_types` gated by each import's constructor visibility:
    /// a file's OWN types + CONCRETELY-imported types bring their ctors; an ABSTRACTLY-imported type
    /// (handle only) brings none; a PARTIALLY-concrete import brings only its named ctors. A ctor name
    /// absent here is not a visible bare variant in that file. A variant whose name COLLIDES with a
    /// prelude type/module name (`Int`/`Bool`/`List`/…) is DELIBERATELY omitted so bare `Int` stays the
    /// width constructor (mirrors `variant_ctor_index`); such a ctor is reached only QUALIFIED, so it
    /// lives in `visible_ctors_qualified` instead — see there.
    visible_ctors: Vec<crate::fxhash::FxHashMap<String, StructId>>,
    /// Per file, EVERY admitted VARIANT-CONSTRUCTOR name → its ctor occurrence — the same admission
    /// gate as `visible_ctors` but WITHOUT the prelude-collision omission. A prelude-named variant
    /// (`Ast.Int`) is unreachable BARE (would shadow the width type) yet perfectly reachable QUALIFIED,
    /// so it belongs to this file's public constructor surface. This is the surface the CDZ0214 withheld
    /// check (`file_scoped_variant_ctor_qualified`) and `is_abstract_type_at` consult for a QUALIFIED
    /// `(. T A)`: `T`'s handle visible + `A` a genuine variant of `T` but `A` NOT here = a withheld
    /// constructor (an abstract or partially-concrete import). Superset of `visible_ctors`.
    visible_ctors_qualified: Vec<crate::fxhash::FxHashMap<String, StructId>>,
    /// Per file, WHOLE-MODULE ALIAS imports (`(import "path" alias)`) → the SPLICED index of the aliased
    /// module. A local name here is NOT a value/type binding but a MODULE HANDLE: `(. alias member)`
    /// projects `member` against that module's exports (`resolve_member`), the collision-free path for a
    /// uniformly-named export imported from 2+ modules. Empty for a single-file / alias-free program.
    module_aliases: Vec<crate::fxhash::FxHashMap<String, usize>>,
}

impl FileScopeTable {
    /// The file whose demux range contains `id`, or `None` for a node in no file (synthesized /
    /// β-copied / prelude / the `(do …)` root).
    ///
    /// BINARY SEARCH over the file ranges, not a linear `position`: files are appended sequentially at link
    /// (each `struct_base = structure.len()` at its turn), so `self.files` is sorted by `struct_base` with
    /// non-overlapping contiguous `[struct_base, struct_base+struct_count)` ranges. This is called on EVERY
    /// file-scoped resolution (`file_scoped_def`/`_type`/`_variant_ctor`), so a linear scan made a package
    /// of N files O(files) per lookup → O(N²) over the package's resolutions (N libraries each imported +
    /// used: `file_of` was ~27% of a 800-file compile). Find the rightmost file whose `struct_base <= id`
    /// (the only candidate range that can contain `id`), then confirm `contains` (an `id` in a gap — the
    /// link-synthesized `(do …)` root, which sits outside every file's range — matches no file).
    fn file_of(&self, id: StructId) -> Option<usize> {
        // `partition_point` gives the count of files with `struct_base <= id.0`; the last of those is the
        // only range that can contain `id` (ranges are ascending + non-overlapping). Subtract one for its
        // index; 0 means `id` precedes every file's base (no file).
        let after = self.files.partition_point(|f| f.struct_base <= id.0);
        let idx = after.checked_sub(1)?;
        self.files[idx].contains(id).then_some(idx)
    }

    /// The `link` path of file index `fi` (see [`Db::file_path`]).
    fn file_path(&self, fi: usize) -> Option<&str> {
        self.files.get(fi).map(|f| f.path.as_str())
    }

    /// Resolve an exported name to the VALUE def it names, WITHIN the file that declares the
    /// `(export …)` clause — the file-scoped analogue of the flat `def_of_name` used by
    /// `scan_top_level`. `name_occ` is the exported NAME atom (which file's demux range it falls in
    /// IS the exporting file); `name` is the name it exports. Returns the def index visible under that
    /// name in the exporting file's OWN scope (its own defs + its imports), or `None` if the file
    /// exports no such value def (a type-handle export, or a name it does not actually define/import).
    ///
    /// This is the ONE resolution site the package linker missed: internal CALLS already resolve
    /// per-file (`file_scoped_def`), but the EXPORT boundary bound through the package-wide first-wins
    /// `def_of_name` — so a same-named def in a sibling file (even a private one, spliced earlier)
    /// hijacked the entry's exported name, running the wrong file's code (`DESIGN-package-linking.md`
    /// §4; `package-export-boundary-binds-flat-def-by-name-not-per-file`). Resolving the export in the
    /// clause's own file — never a sibling's scope — is exactly the per-file rule the call path uses.
    fn export_def(&self, name_occ: StructId, name: &str) -> Option<usize> {
        let file = self.file_of(name_occ)?;
        self.visible[file].get(name).copied()
    }
}

/// Build the file-scoped resolution table from a package's [`crate::link::Linkage`]. For each file:
///  - its OWN top-level value defs — a def whose signature occurrence (`Def::sig_occ`) falls in that
///    file's demux range;
///  - its IMPORTS — each `(import "p" (name…))` local name mapped to the def index the exporting file
///    `p` defines under that name (the link step already verified `p` exports it).
///
/// A def-name lookup against this table is thus confined to one file's surface, which is what makes a
/// sibling file's same-named def invisible unless explicitly imported (`DESIGN-package-linking.md` §4).
fn build_file_scope(
    linkage: &crate::link::Linkage,
    defs: &[Def],
    def_by_name: &crate::fxhash::FxHashMap<String, usize>,
    type_decls: &[TypeDecl],
    prelude_type_module_names: &crate::fxhash::FxHashSet<String>,
) -> FileScopeTable {
    let files = linkage.files.clone();
    let mut visible: Vec<crate::fxhash::FxHashMap<String, usize>> =
        vec![crate::fxhash::FxHashMap::default(); files.len()];
    let mut visible_types: Vec<crate::fxhash::FxHashMap<String, StructId>> =
        vec![crate::fxhash::FxHashMap::default(); files.len()];
    let mut visible_ctors: Vec<crate::fxhash::FxHashMap<String, StructId>> =
        vec![crate::fxhash::FxHashMap::default(); files.len()];
    // The QUALIFIED-access surface: every admitted ctor, INCLUDING prelude-named ones (`Ast.Int`) that
    // `visible_ctors` omits so bare `Int` stays the width type. Reachable only via `(. T A)`, so the
    // CDZ0214 withheld check + `is_abstract_type_at` consult THIS, never the bare map.
    let mut visible_ctors_qualified: Vec<crate::fxhash::FxHashMap<String, StructId>> =
        vec![crate::fxhash::FxHashMap::default(); files.len()];

    // Own defs: assign each def to the file whose range contains its signature occurrence. First-wins
    // per (file, name) mirrors the flat `def_name_index` (a duplicate name within a file is a separate
    // well-formedness concern; here we just pick the first, deterministically).
    for (i, d) in defs.iter().enumerate() {
        if let Some(fi) = files.iter().position(|f| f.contains(d.sig_occ)) {
            visible[fi].entry(d.name.clone()).or_insert(i);
        }
    }

    // Own TYPES: assign each `(type …)` declaration to the file whose range contains its declaration
    // occurrence, binding the type NAME → its synth record and each of its variant CONSTRUCTOR names →
    // the ctor node. A file that declares a type may name it and construct/match its variants; a sibling
    // that did NOT declare it and did NOT import it cannot (the privacy this closes). The
    // prelude-collision skip on bare ctors mirrors `variant_ctor_index` (a variant named `Int`/`List`/
    // `Name` is reached only qualified, never via the bare-name map). `add_type_to_file` (below) is the
    // shared helper the import phase reuses so an imported type is registered EXACTLY as an own one.
    let add_type_to_file =
        |fi: usize,
         decl: &TypeDecl,
         local_name: &str,
         ctor_vis: Option<&crate::link::CtorVis>,
         visible_types: &mut Vec<crate::fxhash::FxHashMap<String, StructId>>,
         visible_ctors: &mut Vec<crate::fxhash::FxHashMap<String, StructId>>,
         visible_ctors_qualified: &mut Vec<crate::fxhash::FxHashMap<String, StructId>>| {
            if let Some(synth) = decl.synth {
                visible_types[fi]
                    .entry(local_name.to_string())
                    .or_insert(synth);
            }
            for v in &decl.variants {
                // Bring this variant's ctor into the file only if `ctor_vis` admits it. `All` (own
                // decl, or a wildcard `T.*` import) brings every ctor; `Named(set)` brings only the
                // explicitly-exported ctors; a `None` `ctor_vis` (an ABSTRACT import — handle only)
                // brings none, so the importer cannot construct or bare-match the type.
                let admit = match &ctor_vis {
                    None => false,
                    Some(crate::link::CtorVis::All) => true,
                    Some(crate::link::CtorVis::Named(set)) => set.iter().any(|n| n == &v.name),
                };
                if admit && let Some(ctor) = v.ctor {
                    // The QUALIFIED surface admits EVERY visible ctor — a prelude-named one (`Ast.Int`)
                    // is reachable through the `(. T A)` path even though bare `Int` is not.
                    visible_ctors_qualified[fi]
                        .entry(v.name.clone())
                        .or_insert(ctor);
                    // The BARE surface omits a prelude type/module-name collision (`Int`/`List`/…) so
                    // bare `Int` keeps resolving to the width type — such a variant is qualified-only
                    // (mirrors `variant_ctor_index`).
                    if !prelude_type_module_names.contains(&v.name) {
                        visible_ctors[fi].entry(v.name.clone()).or_insert(ctor);
                    }
                }
            }
        };
    // Own types: a file sees its OWN declarations concretely (handle + all constructors) — opacity is a
    // BOUNDARY property, never restricting a type within its declaring file.
    for decl in type_decls {
        if let Some(fi) = files.iter().position(|f| f.contains(decl.occ)) {
            add_type_to_file(
                fi,
                decl,
                &decl.name,
                Some(&crate::link::CtorVis::All),
                &mut visible_types,
                &mut visible_ctors,
                &mut visible_ctors_qualified,
            );
        }
    }

    // Imports: bind each imported local name to the def the exporting file defines under the exported
    // name. Resolved via the exporting file's OWN visible map (built above), so an import re-exports
    // exactly the sibling's def — no global scan. (`def_by_name` is the package-wide first-wins index,
    // used only as a last resort if the exporting file's own map somehow lacks it — it will not for a
    // well-formed export, since the link step validated visibility.)
    //
    // An import name may name a VALUE def OR a TYPE (the export list is one namespace-agnostic surface —
    // `modules-and-namespaces.md` §Visibility Is Explicit). So each imported name is tried BOTH ways: if
    // the exporting file declares a type under the exported name, bring its handle + constructors into
    // the importing file (via `add_type_to_file`); if it defines a value def, bind that. A name may be
    // both (rare); both bindings are made and the resolver's ordered steps disambiguate by position.
    let mut module_aliases: Vec<crate::fxhash::FxHashMap<String, usize>> =
        (0..linkage.scopes.len())
            .map(|_| Default::default())
            .collect();
    for (fi, scope) in linkage.scopes.iter().enumerate() {
        for imp in &scope.imports {
            // WHOLE-MODULE ALIAS import (`(import "path" alias)`): record the alias as a module HANDLE for
            // `resolve_member` to project against `from_file`'s exports — NOT a flat value/type binding.
            if imp.module_alias {
                module_aliases[fi].insert(imp.local.clone(), imp.from_file);
                continue;
            }
            // Type import: the exporting file's own `(type …)` under the exported name.
            if let Some(decl) = type_decls.iter().find(|d| {
                d.name == imp.exported
                    && files.get(imp.from_file).is_some_and(|f| f.contains(d.occ))
            }) {
                // The exporting file's constructor visibility for this type decides which ctors cross.
                // `None` (the type is exported but NOT in `type_ctor_exports`) = ABSTRACT: bring the
                // handle only, so a `(. T A)` construction / bare-ctor use in this file is a CDZ0214 (the
                // ctor is hidden on purpose, `withheld_ctor_reject` — a visible handle but no visible
                // ctor), distinct from an unbound name. `Named`/`All` bring the named / all ctors.
                let ctor_vis = linkage
                    .scopes
                    .get(imp.from_file)
                    .and_then(|s| s.type_ctor_exports.get(&imp.exported));
                add_type_to_file(
                    fi,
                    decl,
                    &imp.local,
                    ctor_vis,
                    &mut visible_types,
                    &mut visible_ctors,
                    &mut visible_ctors_qualified,
                );
            }
            // Value import: the exporting file's def under the exported name.
            let idx = visible
                .get(imp.from_file)
                .and_then(|m| m.get(&imp.exported).copied())
                .or_else(|| def_by_name.get(&imp.exported).copied());
            if let Some(idx) = idx {
                visible[fi].insert(imp.local.clone(), idx);
            }
        }
    }

    FileScopeTable {
        files,
        visible,
        visible_types,
        visible_ctors,
        visible_ctors_qualified,
        module_aliases,
    }
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
    /// The specific NAME atom this export came from within the clause — `(export a b)` yields two exports
    /// sharing one `occ` (the clause) but each carrying its own `name_occ` (the `a` / `b` atom). A
    /// diagnostic that points at (or edits) a single exported name uses THIS, not `occ`'s first element,
    /// so a fault on the 2nd+ name of a multi-name clause anchors correctly.
    pub name_occ: StructId,
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
    /// The synthesized CONSTRUCTOR record occurrence — the `ctor` node `sums::synthesize` builds for this
    /// variant (the value of the variant's field in the sum record). `None` until `synthesize` runs;
    /// `Some` once the `Db` exists. Cached here so a per-arm ctor lookup is O(1) instead of re-scanning
    /// the sum record's variant fields by name (`variant_ctor_field`) — that scan was O(V) per match arm,
    /// so a match over a V-variant sum was O(V²). This is the identical `StructId` `variant_ctor_field`
    /// would return; it is captured at build time where it is already in hand.
    pub ctor: Option<StructId>,
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
    /// The OPEN-TAIL row variable, if this sum is declared OPEN — `(type T (Known Int64) .. r)` carries
    /// `Some("r")`. `None` = a CLOSED sum (the mandatory default, `type-system.md §208`: "a sum declared
    /// without a row variable is closed"). An OPEN sum's variant set is not fixed — the row variable `r`
    /// stands for variants the module does not name (arriving across a boundary), so a match over it MUST
    /// carry an open-tail (`_`) arm to be exhaustive (§206), and that `_` is never CDZ0213-redundant. The
    /// runtime representation is unchanged (a tagged value like any sum); open-ness is purely a
    /// compile-time typing/exhaustiveness property. `resolve.rs` still rejects an undeclared bare ctor
    /// CDZ0101 — the row variable does NOT sanction undeclared local ctor names.
    //= spec/capabilities/type-system.md#a-sum-type-may-be-open-with-a-mandatory-open-tail-arm
    //# A program MUST be able to declare an open sum — a variant set that MAY carry a row variable standing for variants not named — so that a value can range over an extensible vocabulary of variants the declaring module does not close, dual to an open record's extensible field set.
    //= spec/capabilities/type-system.md#a-sum-type-may-be-open-with-a-mandatory-open-tail-arm
    //# A closed sum MUST remain the default: a sum declared without a row variable is closed, and the abstract syntax tree type MUST be a closed sum, so that a compiler's match over the AST is checked against a fixed, known variant set.
    pub open_tail: Option<String>,
    /// The SYNTHESIZED record occurrence — the record (`crate::sums`) whose `(meta t)` is this sum's
    /// type-value and whose fields are its variant constructors. `None` until `sums::synthesize` runs
    /// during `Db::load`; ALWAYS `Some` once the `Db` exists. This is what the type NAME resolves to (a
    /// `Ref` to it), reached by the ordinary scope→top-level lookup like a `def` — no separate name map
    /// (which would collapse identity onto the name and clobber a same-named declaration).
    pub synth: Option<StructId>,
    /// ASSOCIATED FUNCTIONS — extra NON-ctor member fields the sum synthesis (`sums::sum_record`) appends
    /// to this sum's record (`(name <op-record>)` nodes), so they are reached as `(. Type member)` like a
    /// variant ctor. Data-driven: a decl declares its own associated members here, and `sum_record` appends
    /// whatever it carries — so a built-in prelude sum's namespaced operations (e.g. `Ast.module`
    /// self-reflection, later `Ast.print`/`Ast.read`) live in the PRELUDE (`sums::prelude_decls` attaches
    /// them, built by `prelude`), NOT a `Db::load` post-synthesis special-case. Empty for a user sum and for
    /// most prelude sums; only a sum with prelude-declared associated ops (currently the built-in `Ast`)
    /// carries any. Nothing is privileged BY NAME in `sum_record` — it just appends the decl's list.
    pub associated: Vec<StructId>,
}

/// One operation of an effect declaration — a `(op NAME (-> Param Result))` clause. Its position in
/// [`EffectDecl::ops`] is its stable operation index (declaration order), the analogue of a variant's
/// discriminant. An effect's operations are a closed, statically-known SET (like a sum's variants or a
/// record's fields), so a duplicate operation name is ill-formed (CDZ0201).
#[derive(Clone, PartialEq, Debug)]
pub struct OpDecl {
    /// The operation's declared name (`emit` in `(op emit (-> Int64 Unit))`).
    pub name: String,
    /// The operation's NAME occurrence — the `op` clause's name atom. A duplicate-operation diagnostic
    /// anchors here.
    pub name_occ: StructId,
    /// The operation's TYPE occurrence — the `(-> Param Result)` arrow written after the name. Copied
    /// into the synthesized op-value's `(meta t)` so a performance `(E.op a)` types as an ordinary
    /// application against that arrow (`capabilities-and-effects.md` §Performing An Operation Is Typed).
    /// `None` for a malformed `(op NAME)` with no type.
    pub ty: Option<StructId>,
    /// The 0-based PARAMETER INDEX the `@resource` marker designated as this op's SEC-F1 resource (the
    /// destination the kernel/executor routes on + Cedar authorizes), or `None` for a target-free op. The
    /// `@resource T` surface marker lifts (hash-clean, stripped from the op TYPE) to a `(resource <idx>)`
    /// decl-level sibling on the op node (v-syntax 2026-08-13); this reads that sibling. Load-bearing for
    /// the schema-hash reify: a reducer's world-effect perform reifies its args into the effect-request
    /// record, routing the resource arg to its own `target` wire field (v-rb ruling A, 2026-08-14) — the
    /// dest is a RUNTIME VALUE (SEC-F1 authorizes it against a resource predicate), so it rides the wire,
    /// NOT dropped. The resource arg is skipped from the PAYLOAD-encode only, so a `(@resource dest, payload)`
    /// perform reifies with the payload = the single remaining arg (no in-fold value-encode / R2 needed for
    /// the common one-resource-one-payload op) and `target` = the bare dest bytes. `None` = a target-free op
    /// (model/now/timer/tool) → no `target` field, a 3-field {correlation, kind, payload} record.
    pub resource: Option<usize>,
}

/// An `(effect NAME (op f (-> A B)) …)` declaration scanned from the top level — a routing-agnostic
/// contract naming an effect and typing each of its operations (`capabilities-and-effects.md` §An Effect
/// Declaration Names The Effect And Types Its Operations). Modeled as the effect analogue of a
/// [`TypeDecl`]: identity is the declaration OCCURRENCE (not the name), so two effects that declare a
/// same-named operation never collide (the op is reached through its effect). Its `synth` record —
/// fields = operation values, like a sum record's fields = variant constructors — is built by
/// `effects::synthesize`, so `E.op` is ordinary member access and nothing is privileged by name.
#[derive(Clone, PartialEq, Debug)]
pub struct EffectDecl {
    /// The effect's name (`E` in `(effect E …)`).
    pub name: String,
    /// The declaration occurrence — the effect's NOMINAL IDENTITY (the arena occurrence, as `TypeDecl`)
    /// and the anchor for a declaration-level diagnostic. Two `(effect Foo …)` declared separately are
    /// DISTINCT effects even with the same name.
    pub occ: StructId,
    /// The operations in declaration order — each position's index is its stable operation index.
    pub ops: Vec<OpDecl>,
    /// The SYNTHESIZED record occurrence — the record whose `(meta t)` marks it an effect and whose
    /// fields are its operation values. `None` until `effects::synthesize` runs during `Db::load`;
    /// ALWAYS `Some` once the `Db` exists. This is what the effect NAME resolves to (a `Ref` to it),
    /// reached by the ordinary scope→top-level lookup like a `def`/`type` — no separate name map.
    pub synth: Option<StructId>,
}

// The overflow-POLICY boundary types now live in the shared `cadenza-compile-abi` crate (the compile-
// boundary contract `cdz` and `rcdzc` agree on — `cdz` builds the spec from the manifest + threads it
// across the delegate seam, `rcdzc` reads it here). Re-exported so `crate::db::{OverflowMode,
// OverflowSpec}` / `rcdzc::db::…` and every internal ref stay byte-stable.
pub use cadenza_compile_abi::overflow::{OverflowMode, OverflowSpec};

/// A `(module NAME def…)` declaration reachable in a `do`-block — a namespace binding `NAME` to a record
/// of its exported definitions (`core-semantics.md` §A Module Groups Definitions Under A Name). Its
/// members are reached by member access `(. NAME field)`, exactly like a sum's variants or an effect's
/// operations. Identity is the declaration occurrence (like `TypeDecl`/`EffectDecl`); two `(module Foo …)`
/// declared separately are distinct.
#[derive(Clone, PartialEq, Debug)]
pub struct ModuleDecl {
    /// The module's name (`m` in `(module m …)`).
    pub name: String,
    /// The declaration occurrence — the module's identity + the anchor for a declaration-level diagnostic.
    pub occ: StructId,
    /// The SYNTHESIZED record occurrence — the record whose fields are the module's exported defs (a value
    /// def's body, a function def's lambda). `None` until `modules::synthesize` runs during `Db::load`;
    /// ALWAYS `Some` once the `Db` exists. The module NAME resolves to a `Ref` to it, reached by the
    /// ordinary scope→lookup order like a `def`/`type` — no separate name map, nothing privileged by name.
    pub synth: Option<StructId>,
    /// The TYPE-EXPRESSION occurrence of a `(pragma default-integer <T>)` member, if the module declares
    /// one — the type an otherwise-unconstrained bare integer literal WRITTEN in this module defaults to,
    /// instead of `Int64` (`numeric-model.md` §A Module May Declare Its Default Integer Literal Type).
    /// `None` for a module with no such pragma. Kept as the un-reduced occurrence (registration runs
    /// before the evaluator exists); the load-time `default_int_literals` map records which literals it
    /// applies to (a per-node map, robust to the β-copy reparenting a parent-walk cannot follow).
    pub default_int: Option<StructId>,
    /// The TYPE-EXPRESSION occurrence of a `(pragma default-fraction <T>)` member, if the module declares
    /// one — the exact-rational type an otherwise-unconstrained NUMERIC literal (integer OR decimal)
    /// WRITTEN in this module grounds to, so arithmetic in the module is exact by default
    /// (`numeric-model.md` §A Module May Declare Its Default Fraction Literal Type). `None` for a module
    /// with no such pragma. Like `default_int`, kept as the un-reduced occurrence; the load-time
    /// `default_fraction_literals` map records which literals it applies to.
    pub default_fraction: Option<StructId>,
    /// The TYPE-EXPRESSION occurrence of a `(pragma default-float <T>)` member, if the module declares one
    /// — the floating-point type an otherwise-unconstrained bare DECIMAL literal WRITTEN in this module
    /// defaults to, instead of `Float64` (`numeric-model.md` §A Module May Declare Its Default Float
    /// Literal Type). The float twin of `default_int`. `None` for a module with no such pragma. Kept as the
    /// un-reduced occurrence; the load-time `default_float_literals` map records which literals it applies to.
    pub default_float: Option<StructId>,
    /// The overflow policy of a `(pragma overflow (signed <mode>) (unsigned <mode>))` member, if the module
    /// declares one — the trap/wrap behavior an unqualified `+`/`-`/`*` WRITTEN in this module takes,
    /// selected by operand signedness (`numeric-model.md` §A Module May Declare Its Overflow Policy). `None`
    /// for a module with no such pragma. Applied at load by the per-node `overflow_specs` map (β-copy-robust,
    /// like the default-literal maps); the signed-vs-unsigned selection is resolved in `infer`.
    pub overflow: Option<OverflowSpec>,
}

/// The bound on β-reduction nesting depth — a backstop. A recursive function is caught STATICALLY
/// (a call whose callee transitively references itself declines before any inlining — see
/// `eval::is_recursive`), so this only guards a non-recursive fold that nests unexpectedly deep. Kept
/// low: a recursive function that branches (several self-calls per body) would explode EXPONENTIALLY
/// in node appends well before a high depth, so the static check is the real guard and this is a
/// cheap safety net. No legitimate compile-time fold nests this deep.
pub(crate) const REDUCE_DEPTH_LIMIT: u32 = 32;

/// The bound on the TOTAL number of β-reductions the evaluator may ATTEMPT in one compile — the
/// cumulative-WORK budget that complements [`REDUCE_DEPTH_LIMIT`]'s per-fold DEPTH budget. A term can
/// stay within the depth limit yet drive an EXPONENTIAL number of bounded reductions: a self-applying
/// lambda `(fn (v) (v (v v)))` applied to itself is not statically recursive (it calls a PARAMETER, so
/// `is_recursive` finds no call-graph cycle) and each reduction stays shallow, but the type/fault walk
/// over the shared, doubling term attempts ~2^depth reductions — the compiler does unbounded work and
/// appears to hang (the `cdz-smith` timeout). Every reduction funnels through [`Db::enter_reduction`],
/// which counts entries against this budget and denies past it, so the reduction DECLINES rather than
/// diverges — the same decline a too-deep fold gets. Set FAR above any legitimate compile (no corpus
/// program attempts even 20 000 reductions — a real fold's total is linear in its inlined size), yet an
/// explosive term crosses it in a fraction of a second, so a valid program never hits it and a diverging
/// one declines promptly instead of hanging. Reset per `Db` (a fresh compile); never decremented.
pub(crate) const REDUCE_NODE_BUDGET: u64 = 1_000_000;

/// PER-DEF-BODY bound on STRUCTURAL reductions (the `Ref`/nominal-unwrap hops through
/// [`enter_reduction_structural`], which — unlike the β-reduction [`enter_reduction`] — is NOT charged
/// against [`REDUCE_NODE_BUDGET`], so a term that explodes through the structural path alone escapes that
/// guard). A self-application whose type/fault walk re-reduces an exponentially-WIDENING term
/// (`((fn (v0) (v0 (- (fn v1) (v0 v0)))) …)`) drives UNBOUNDED structural reductions in ONE def body while
/// staying within β-depth — the `cdz-smith` type-inference HANG escaping CDZ0999. This counts structural
/// reductions PER DEF BODY (reset in `collect_faults`' body loop); the count grows WITH the exploding width
/// so it trips BEFORE the width materializes (fast decline), while a real def body's structural reductions
/// stay far below it (v-compiler-perf's volume data: the whole compile is ~800k CUMULATIVE across ALL defs,
/// so any one body is a small fraction — this per-body ceiling has a wide margin). Past it,
/// `enter_reduction_structural` denies entry (declines like the depth limit), so the walk bottoms out with a
/// resource-limit CDZ0999 instead of hanging. (Tunable; v-compiler-perf gates the self-host carve-out.)
///
/// RAISED 2M → 8M (#6100 shipped 2M, but that FALSE-DECLINED a legitimate large EFFECT-FOLD body): the
/// 14c corpus case `lgt1` (a two-op `#tuple`-state handler, 8 perform sites, many nested-match resumes) does
/// ~3–4M structural reductions in one def body during its fault walk, tripped the 2M ceiling mid-fold, and
/// declined codeless "value is not applyable" (a pass→todo regression git-bisected to #6100). Bracketed:
/// 2M/2.5M/3M decline, 4M/200M compile — so lgt1's peak is ~3–4M; 8M gives a ~2× margin. A RAISE is
/// monotonic-safe for correctness (it only ADMITS more reductions — a program that compiled at 2M is
/// unchanged; one that declined either still declines above 8M or now compiles).
///
/// WHY 8M and not higher: the ceiling's cost is the HANG-DECLINE LATENCY, which scales ~LINEARLY with it —
/// each structural reduction is ~constant work and the counter must REACH the ceiling before
/// `enter_reduction_structural` denies entry, so a self-app HANG case that declines in ~2s at 2M takes ~4×
/// that at 8M (and ~8× at 16M). Every corpus gate-local runs these self-app decline cases, so a looser
/// ceiling is a recurring gate cost — 8M is the smallest power-of-two clearing lgt1's ~4M peak with a safe
/// margin while keeping that per-decline cost low (v-compiler-perf's carve-out-gate measurement, #6100
/// owner; supersedes an earlier 16M whose cost reasoning wrongly assumed the HANG trips in "a few cheap
/// doublings"). Validated at 8M by v-compiler-perf (full rcdzc lib suite 1459/0; both `*_inference_hang`
/// regression tests + the Option.None `*_survives_reduction_budget_exhaustion` carve-out all pass). If a
/// 14c/14b sibling effect-fold body ever exceeds 8M, raise the const further (the fix is just this value).
pub(crate) const STRUCTURAL_REDUCTION_BUDGET: u64 = 8_000_000;

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

/// The bound on a LAYOUT core-tree WALK's own recursion depth ([`Db::walk_depth`]) — the backstop against a
/// native stack overflow when a walk (`collect_call_callees`, `collect_closure_codes`) descends a deep
/// memoized `Core::SumNew`/compound chain a non-normalizing self-application materializes. Separate from
/// [`DESCENT_DEPTH_LIMIT`] because a walk sits ON TOP of `core_of`'s own descent (each walk node calls
/// `core_of`), so the two counters must not share — but the same magnitude is right (both bound native
/// recursion against the same worker stack, sized in `host.rs` from `DESCENT_DEPTH_LIMIT`). A valid program
/// never nests core this deep (`core_of` would itself decline it first); a pathological chain is clipped
/// here so `layout::compute` completes and `collect_faults` reports the coded decline.
pub(crate) const WALK_DEPTH_LIMIT: u32 = 1024;

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

/// The call-site index (`Db::call_sites_by_callee`): `callee def index → the (caller-body, argument-
/// occurrences) of every application calling that callee`. Named to keep the field type legible.
pub(crate) type CallSiteIndex = crate::fxhash::FxHashMap<usize, Vec<(StructId, Vec<StructId>)>>;

/// A cached record-field "did you mean?" result (`Db::no_field_suggestion`): the confident single winner
/// (drives the fix, `None` when the typo is too far) and the full hint-message suffix (`— did you mean …`
/// or `— closest matches: …`).
pub(crate) type NoFieldSuggestion = (Option<String>, String);

/// A cached record-type field index (`Db::record_field_index`): a `guard` `Rc` pinning the record type's
/// `BTreeMap` allocation alive (so its `as_ptr` key cannot be reused) plus the `field-name → sorted-slot`
/// map built once from its sorted keys. Reading `index.get(key)` replaces the O(fields) `.position()` scan.
pub(crate) struct RecordFieldIndex {
    /// Held ONLY to keep the record type's `BTreeMap` allocation alive so its `as_ptr` cache KEY cannot be
    /// reused by a later, different `BTreeMap` (ABA safety) — never read directly.
    #[allow(dead_code)]
    pub(crate) guard:
        std::rc::Rc<std::collections::BTreeMap<crate::resolved::Symbol, crate::ty::Ty>>,
    pub(crate) index: crate::fxhash::FxHashMap<crate::resolved::Symbol, usize>,
}

/// A cached SIMPLE list-pattern binder index (`Db::simple_list_binders`): for a `(list p0 … .. rest)`
/// pattern whose every leading element is a bare-name binder (`_` and `..` excluded), the `binder-name →
/// its access path` from the list scrutinee. A leading binder `pi` reads element `i` (`[Elem(i)]`); the
/// rest binder reads the tail sublist from `lead` onward (`[RestFrom(lead)]`). No leading binder carries
/// a variant head (a bare name is not a variant payload), so a heads vector is never needed — the paths
/// are pure `Elem`/`RestFrom` steps. Built once per pattern; `get` is an O(1) `FxHashMap` read replacing
/// the O(leading) per-reference scan. A pattern with ANY nested (tuple/variant/list) leading element is
/// NOT simple — it is absent from `Db::simple_list_binders` as `None` and the caller falls back to the
/// exact linear descent (`find_binder_in_*`), so this fast index never replicates the nested enumeration.
pub(crate) struct SimpleListBinders {
    /// `binder-name → the access-path steps` for every leading bare-name binder and the rest binder.
    pub(crate) by_name: crate::fxhash::FxHashMap<String, Vec<crate::core::PathStep>>,
}

/// A conservative fact about the value at a Core occurrence, in the current flow context. A `ValueFact`
/// is a product of independent, individually-optional FACETS so the domain is extensible ("add a facet"
/// ≠ "rewrite the domain"): every field defaults to "unknown" (⊤, the safe default), and each is joined
/// independently at a control-flow merge (per-var O(1), never cross-variable — a relational domain is
/// deliberately OUT of the foundation).
///
/// SOUNDNESS (load-bearing — check elision defaults to KEEPING the check): a `ValueFact` is a conservative
/// OVER-approximation of the set of values the occurrence may take here, i.e. `actual ⊆ fact`. A check is
/// elided only when the fact PROVES it dead (e.g. div-by-zero elided iff `0 ∉ fact`). So a fact that is
/// too WIDE (still a superset of `actual`) is SAFE — it may fail to prove the check dead, costing only a
/// missed optimization; a fact that is too NARROW (drops a real value like 0) is UNSOUND — it would
/// wrongly prove the check dead and elide a needed guard. Therefore the JOIN (at control-flow merges)
/// WIDENS (set-union of possibilities) and the MEET (at a refinement) NARROWS (intersection): a
/// refinement may only shrink the set to values the branch condition GUARANTEES.
///
/// Slice 1 populates `int_range` (a behavior-identical port of the former bare `(i64, Option<i64>)`
/// interval tuple); the bounded index<len bounds facet (operator-greenlit) adds `below_len` (a RELATIONAL
/// fact: this index is `< len(collection)`); the remaining facets are reserved for later slices (nonzero →
/// div-by-zero elision) and are always ⊤ today. `variant_tags` was tried and REMOVED — the lower tier
/// already absorbs same-scrutinee nested-match discriminant elision (slice-5, 7828dbd9c).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ValueFact {
    /// Signed integer interval `[lo, hi]` (`hi = None` ⇒ unbounded above) KNOWN to hold here — SUBSUMES the
    /// former `(i64, Option<i64>)` refinement tuple. `None` ⇒ no interval fact (⊤).
    pub(crate) int_range: Option<(i64, Option<i64>)>,
    /// The BOUNDED-INDEX facet (operator-greenlit bounds-elision): the binder of a COLLECTION whose LENGTH
    /// this value is KNOWN to be strictly below in the current branch — established by an enclosing
    /// `(< i (List.len xs))` guard (then-branch), which proves `i < len(xs)`. A `List.at xs i` under this
    /// fact can shed its OWN redundant `index < len` bounds check (the enclosing guard already proved it),
    /// exactly as `int_range`'s non-negativity sheds the `index >= 0` half. `None` ⇒ no below-len fact (⊤).
    /// KEYED ON COLLECTION IDENTITY: a fact for `xs` never licenses eliding a check on a DIFFERENT list —
    /// the consumer matches the accessed list's binder, so a guard on `ys` cannot silently drop `xs`'s
    /// check (that would be an out-of-bounds read). This is a RELATIONAL fact (index-binder → collection-
    /// binder), NOT an interval, so it joins by IDENTITY-equality (a control-flow merge keeps it only when
    /// BOTH sides agree on the same collection — see `join`).
    pub(crate) below_len: Option<StructId>,
}

impl ValueFact {
    /// The interval-only fact — the slice-1 constructor mirroring the former bare tuple.
    pub(crate) fn from_int_range(lo: i64, hi: Option<i64>) -> ValueFact {
        ValueFact {
            int_range: Some((lo, hi)),
            below_len: None,
        }
    }
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
    /// The top-level `(effect …)` declarations, from the scan (their operation sets validated at
    /// well-formedness time — a duplicate operation name is CDZ0201). The effect analogue of
    /// `type_decls`; identity is the declaration occurrence.
    pub effect_decls: Vec<EffectDecl>,
    /// The interface name this component publishes its exports under, when compiled AS A PROVIDER (X4b) —
    /// from the `component-name` compile-request artifact. `Some("cadenza:pkg/iface")` → `emit` wraps the
    /// boundary exports as that named interface instance so a peer's `(effect …)` `(bind "cadenza:pkg/iface")`
    /// binds (the effects-unified surface, U2). `None` (the common case) → exports cross as top-level
    /// component funcs (byte-identical to before). Set AFTER `Db::load`/`load_linked` by the caller
    /// (`compile`), which reads the request artifact.
    pub component_name: Option<String>,
    /// Whether this emit will produce DWARF debug info that references function LOCAL SLOTS — set by
    /// `backend::emit` from the target (`wasm-debug`/`dwarf`, or a lean `wasm` paired with a `dwarf`
    /// sidecar). The wasm backend's local-slot COALESCER reads this: when true it PINS debug-named
    /// slots (a `let`-binding / match-binder a DWARF DIE points at) so they keep distinct, correctly
    /// located slots; when false (a plain standalone `wasm` — the shipped + gap-sweep target, no DWARF
    /// consumer) it coalesces ALL non-interfering slots freely (dramatically fewer declared locals on
    /// the effects-lowering blowup). Default `false`.
    pub emit_debug: bool,
    /// PER-FILE pre-resolve SOURCE snapshots, indexed by the SAME file index [`file_of`](Db::file_of)
    /// returns — the comment-stripped raw arena of a linked file, captured in `compile::link_inputs`
    /// BEFORE `Db::load` runs any mutating pass (strip_def_docs / verify / world-import-export injection /
    /// resolve). The self-reflection intrinsic `Ast.module` (`Prim::ReflectModule`) is filled at LOWERING
    /// from these: `core_of` maps the occurrence's `file_of` index to its snapshot and reflects that file's
    /// module root as an `Ast` value (`arenas_to_ast_value`), so the reflected value is byte-identical to
    /// `quote`/`__ast__` over the same module — the live `ast` arena is unusable there (it is mutated in
    /// place before lowering). `None` for a file that does NOT contain an `(. Ast module)` form: the clone
    /// is GATED on actual self-reflection use (`quote::contains_ast_module`), so the overwhelmingly common
    /// program pays NO snapshot clone. `Rc` so the fill can clone one out (O(1)) without borrowing `Db`
    /// immutably while it mutates the arena. Empty for a `Db` built without linkage-time capture (tests via
    /// `Db::load`); set AFTER `load_linked` by `compile`, like `component_name`.
    pub source_snapshots: Vec<Option<std::rc::Rc<Arenas>>>,
    /// The PREPARSED TARGET WIT WORLD as a raw `cadenza-ast` binary document (§3b full-A end-state), or
    /// `None` when the program targets no world. Populated by `compile` from the `KIND_WIT_WORLD` input
    /// artifact (an external WIT→binary-AST step OR v-syntax's inline-declaration lowering — rcdzc never
    /// parses WIT). The world-structure reader (`wit_world`) decodes + walks this to drive per-member
    /// emit-to-match: a member the world declares as a `list<u8>` boundary over a value-encodable guest
    /// compound has its structured body `value-encode`/`value-decode`-wrapped to that ABI (the reducer
    /// fold `apply` is the first such member). This is the SOLE bytes-boundary signal.
    pub wit_world: Option<Vec<u8>>,
    /// EFFECT → PEER-CONTRACT bindings (U2, the effects-unification of cross-component interop): a
    /// top-level `(bind Math "cadenza:math/api")` DEFAULTS the whole component scope to route the escaping
    /// effect `Math` to the peer interface `cadenza:math/api` (rather than the generic host). An escaping
    /// perform of a bound effect's op emits through the PEER envelope (`assemble_extern`) with this
    /// interface, exactly as an `(extern …)` op did. An in-program `(handle Math …)` discharges the effect
    /// BEFORE it escapes (so it overrides the binding — a test mock), and a compile-request `--bind`
    /// override (U3) is merged in AFTER load (so it wins over the source default). Empty for a program
    /// that binds no effect to a peer (the common case — an effect escapes to the host, byte-neutral).
    pub effect_bindings: std::collections::BTreeMap<String, String>,
    /// The nested `(module NAME …)` declarations reachable in a `do`-block, from the scan. Each is
    /// synthesized (`crate::modules`) to a record whose fields are its exported defs, so `NAME` resolves
    /// to that record (a `Ref` to it) and `(. NAME field)` is ordinary member access — the module analogue
    /// of `type_decls`/`effect_decls`, identity the declaration occurrence, nothing privileged by name.
    pub modules: Vec<ModuleDecl>,

    /// For each ERASABLE nominal NEWTYPE declaration (a single-variant sum whose box is erased), its
    /// UNDERLYING structural-type TEMPLATE — the `inner` of the `Ty::Nominal` its `decl` denotes, with
    /// each of the declaration's TYPE PARAMETERS represented as `Ty::Var(i)` (`i` = the param's position
    /// in `TypeDecl::params`). Precomputed ONCE at load (by `infer::newtype_underlying`) so
    /// `resolve::decode_ty` can normalize a `Ty::Sum` naming an erasable decl into a `Ty::Nominal` with
    /// only `&Db` (the predicate needs `&mut`).
    ///
    /// A MONOMORPHIC newtype has no params, so its template IS the concrete inner (`(type UserId (Mk
    /// Int64))` → `Int64`). A GENERIC newtype's template carries the param vars (`(type Box (Mk a))` →
    /// `Ty::Var(0)`), and `decode_ty` SUBSTITUTES the sum's decoded `args` positionally to get the inner
    /// at THIS instantiation (`Box Int64` → `Int64`, `Box Bool` → `Bool`). So one template serves every
    /// instantiation — the generic analogue of the monomorphic inner. A decl NOT in this map stays an
    /// ordinary boxed `Ty::Sum` (multi-variant / recursive / sum-carrying). This is the ONE place the
    /// "which sums erase" decision is materialized; every type reader consults it via `decode_ty`, so the
    /// `Sum`↔`Nominal` choice is uniform across the compiler.
    pub newtype_inner: crate::fxhash::FxHashMap<StructId, crate::ty::Ty>,

    /// Each bare integer-LITERAL node WRITTEN inside a `(module … (pragma default-integer <T>) …)` → the
    /// pragma's `<T>` type-expression occurrence. A literal in this map defaults to `<T>` instead of
    /// `Int64` (`numeric-model.md` §A Module May Declare Its Default Integer Literal Type). Built ONCE at
    /// load by an ARENA walk of each pragma-module's member subtrees (`collect_default_int_literals`) —
    /// keyed by the ORIGINAL source-literal node, so it is robust to the β-copy reparenting a parent-walk
    /// cannot follow (an inlined body's literal keeps its original id, which stays in this map). DEFINITION-
    /// SITE scoped: only literals written in the module carrying the pragma, never a caller's. `infer::
    /// compute` consults it to give the literal its starting type; an explicit annotation still wins (the
    /// `Annot` node fixes its own type) and no-promotion still holds (the default is a type, not a coercion).
    pub default_int_literals: crate::fxhash::FxHashMap<StructId, StructId>,

    /// Each bare NUMERIC-LITERAL node (integer OR decimal) WRITTEN inside a `(module … (pragma
    /// default-fraction <T>) …)` → the pragma's `<T>` type-expression occurrence. A literal in this map
    /// grounds to the exact rational `<T>` instead of Int64/Float64 (`numeric-model.md` §A Module May
    /// Declare Its Default Fraction Literal Type). Built ONCE at load by an ARENA walk
    /// (`collect_default_fraction_literals`), keyed by the ORIGINAL source-literal node so it survives the
    /// β-copy an inlined body performs. DEFINITION-SITE scoped. Unlike `default_int_literals` (integer
    /// literals only), this marks DECIMAL literals too — an exact default applies to `0.5` as `1/2`.
    /// `infer::compute` consults it to type the literal `Ty::Rational`; `lower` folds it to a
    /// `Core::ConstRational` (via `rational_from_literal`); an explicit annotation still wins, and
    /// no-promotion still holds (the default fixes a type, not a coercion).
    pub default_fraction_literals: crate::fxhash::FxHashMap<StructId, StructId>,

    /// Each bare, UN-SUFFIXED integer-literal node WRITTEN as the direct argument of a variant/newtype
    /// constructor whose corresponding DECLARED payload type is `BigInt` — e.g. `42` in `(W 42)` for
    /// `(type W (W BigInt))` or `(Ast.Int 42)`. A literal in this set GROUNDS to `Ty::BigInt` instead of
    /// the `Int64` default (`numeric-model.md`: an integer literal grounds to `BigInt` LOSSLESSLY, and an
    /// explicit context takes precedence over the declared default — so this is a grounding, NOT a
    /// promotion). Built ONCE at load by an ARENA walk (`collect_bigint_ctor_arg_literals`), keyed by the
    /// ORIGINAL source-literal node so it survives the β-copy an inlined body performs. `infer::compute`
    /// consults it to type the literal `Ty::BigInt`; `lower` needs nothing (a `Core::ConstInt` typed
    /// `BigInt` already materializes as a BigInt leaf). DISCIPLINE (operator-confirmed): ONLY a bare,
    /// un-suffixed literal is marked — a suffixed `42N` (already `Ty::BigInt`), an already-typed value, or
    /// a COMPUTED Int64 expression is NEVER marked, so a genuine Int64/BigInt mismatch still declines.
    pub bigint_ctor_arg_literals: crate::fxhash::FxHashSet<StructId>,

    /// LAZY CACHE of every `Resolved::Resume` value + next-state node in the program — the operands a resume
    /// can carry a value OUT through. Built ONCE on first demand by [`crate::infer::binder_resume_escapes`]
    /// (the FIND3 (B) shell-reclaim fence's pre-reduction resume-escape check), then consulted per candidate
    /// scrutinee-binder. Collects TAIL and NON-TAIL resumes alike (a non-tail resume escapes its operands
    /// too), so the fence never misses an escape → no false-negative (a false-negative would be a UAF; a
    /// false-positive is only a leak-safe missed reclaim). `None` until first built; empty once built for a
    /// resume-free program (so the fence is a cheap O(1) miss on pure code). Not populated at load (the
    /// escape check needs `resolved_of`, unavailable during the structural load walk).
    pub resume_escape_operands: Option<Vec<StructId>>,

    /// Each bare DECIMAL-LITERAL node WRITTEN inside a `(module … (pragma default-float <T>) …)` → the
    /// pragma's `<T>` type-expression occurrence. A literal in this map defaults to `<T>` (a float width)
    /// instead of `Float64` (`numeric-model.md` §A Module May Declare Its Default Float Literal Type). The
    /// float twin of `default_int_literals` — but it marks DECIMAL literals only (an integer-written literal
    /// keeps its integer default; a default float width governs how a written-decimal literal grounds, not
    /// how an integer does). Built ONCE at load by an ARENA walk (`collect_default_float_literals`), keyed
    /// by the ORIGINAL source-literal node (β-copy-robust). DEFINITION-SITE scoped. `infer::compute`
    /// consults it to give the literal its starting float type; an explicit annotation still wins and
    /// no-promotion still holds (the default fixes a type, not a coercion — like the integer default, a
    /// float default keeps the value form a `ConstFloat`, so no annotated-literal guard is needed).
    pub default_float_literals: crate::fxhash::FxHashMap<StructId, StructId>,

    /// Each unqualified `+`/`-`/`*` arithmetic-operator NODE WRITTEN inside a `(module … (pragma overflow
    /// …) …)` (or at a root-scope `(pragma overflow …)`'s def bodies) → the governing [`OverflowSpec`] (the
    /// declared signed/unsigned trap-or-wrap pair). The overflow twin of `default_int_literals`: built ONCE
    /// at load by an ARENA walk (`collect_overflow_modes`), keyed by the ORIGINAL operator node so it
    /// survives the β-copy an inlined body performs (the parent-walk a naive lookup would use is unusable
    /// post-copy). DEFINITION-SITE scoped (a nested `(module …)` keeps its own policy). A named
    /// `Int64.wrapping-*` form is never a key (it is not an unqualified `+`/`-`/`*`). `infer` reads this per
    /// node and SELECTS the single [`OverflowMode`] by the operand's concrete signedness (decision B — one
    /// authoritative resolved value per node, so const-fold + backend + oracle cannot drift).
    pub overflow_specs: crate::fxhash::FxHashMap<StructId, OverflowSpec>,

    /// The GLOBAL overflow default from the `Project.cdz` manifest (`overflow-signed`/`overflow-unsigned`) —
    /// the precedence level BELOW a module `(pragma overflow …)` and ABOVE the built-in `Trap` default
    /// (`numeric-model.md` §"Overflow Behavior Is Configurable By Policy": module-pragma > global-manifest >
    /// trap). `None`/`None` (the `Default`) when no manifest default is set → arithmetic traps unless a
    /// module pragma says otherwise. THREADED IN by the manifest ingestion (v-cdz-crate-split owns the
    /// `Project.cdz` parse — `overflow_signed`/`overflow_unsigned` manifest fields); read per node by
    /// `infer::overflow_mode_of` via [`Db::global_overflow_default`] when a node has no governing module spec.
    pub global_overflow: OverflowSpec,

    /// The declaration occurrences of sums that are C-style ENUMS — MORE than one variant, EVERY variant
    /// nullary (no payloads). Such a value carries no data beyond WHICH variant it is, so it needs no heap
    /// box: it is represented at run time DIRECTLY as its discriminant `i32` (construction is a
    /// `ConstI32(disc)`, a match switches on the i32 itself with no `sum-disc` call, equality is `i32.eq`,
    /// and it crosses the host boundary as a component `enum`). A SINGLE-variant nullary sum is a nominal
    /// Unit (in `newtype_inner`, not here); a MIXED sum (some variant carries a payload — `Option`, a
    /// recursive sum) stays a boxed `Ty::Sum` handle so its nullary and payload variants share one uniform
    /// representation. Materialized once at load (like `newtype_inner`), so every backend site agrees on
    /// the representation via `is_enum_disc`.
    pub enum_disc: crate::fxhash::FxHashSet<StructId>,

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

    /// `def signature occurrence → its stripped `(doc "…")` text` — the DOCUMENTATION column the
    /// `DocOf`/`DocAt` queries read. Filled once at load by [`strip_def_docs`], which removes the doc form
    /// from a def's body (so every body reader stays `tail.get(1)`) but records its text here keyed by the
    /// def's signature occurrence. That key is the node both a name lookup (`def_by_name` → `Def::sig_occ`)
    /// and a body lookup (`def_index_by_body` → `sig_occ`) resolve to, so a doc is reachable from either a
    /// definition NAME or a cursor OFFSET. A def with no doc has no entry (`doc_of_def` returns `None`).
    doc_by_sig: crate::fxhash::FxHashMap<StructId, String>,

    /// `def signature occurrence → its index in [`defs`]` — the O(1) reverse of a def's `sig_occ`, backing
    /// `infer::def_of_param`'s "which def owns this parameter?". That was a linear
    /// `defs.iter().position(|d| d.sig_occ == sig)`; the transitive-inference feature (`989e8c7c`) calls
    /// `def_of_param` per parameter reference (`arg_is_other_def_param`), so a wide module made it O(N²)
    /// (8000 defs: `def_of_param` 42% self). LAZILY built + cached (paired with the `defs.len()` it was
    /// built at, so a later synthesized-def push — effect specialization — is picked up by a rebuild;
    /// pushes are rare, so the rebuild is amortized). `None` until first built.
    def_by_sig: Option<(usize, crate::fxhash::FxHashMap<StructId, usize>)>,

    /// `def-IDENTIFYING node → its index in [`defs`]` — the O(1) reverse for `sidecar::def_identified_by`
    /// ("which def does this node HEADER identify?"). A def is identified by three nodes: its `(def …)`
    /// FORM, its signature list `(NAME param…)`, and its NAME atom. That check was a linear
    /// `defs.iter().enumerate()` scan per query; `hover_text`/`TypeAt` runs it per queried node, and
    /// `cdz query --where` issues a `TypeAt` per structural match → O(matches × defs) = O(N²) (6400 matches
    /// over 6400 defs: `run_query`/`def_identified_by` 81% self). Lazily built + length-guarded like
    /// [`def_by_sig`]. Each def contributes ≤3 entries; FIRST-wins on the off-chance two share a node.
    def_by_ident: Option<(usize, crate::fxhash::FxHashMap<StructId, usize>)>,

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
    /// WARNING: FLAT-NAMESPACE ASSUMPTION — revisit when nested modules / imports land. `scan_top_level` is
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

    /// For each user-sum VARIANT NAME, the occurrence of its CONSTRUCTOR record (first-declared wins).
    /// Built once at load from `type_decls[].variants[].ctor`. `variant_ctor_by_name` consults it for
    /// EVERY bare-variant-name reference in `resolve_name` — so a `match` over an N-variant sum resolved
    /// N arm heads, and the old body (scan `type_decls`, then `variant_ctor_field` scan the sum record's
    /// V fields by name) was O(V) per arm → O(N·V) = O(N²) for a full match over one big enum (measured:
    /// 3200 variants ≈ 340ms, ~40% self-time in `variant_ctor_field`'s field scan). This map makes each
    /// lookup O(1). FIRST-wins matches the old `type_decls`-order scan (a variant name shared across two
    /// sums takes the first-declared; a qualified `(. Type Variant)` disambiguates). A pure lookup
    /// ACCELERATOR over `type_decls`, never a source of truth — the ctor's identity is its occurrence.
    variant_ctor_index: crate::fxhash::FxHashMap<String, StructId>,

    /// The variant ctors DELIBERATELY OMITTED from `variant_ctor_index` because their name COLLIDES with
    /// a prelude TYPE/MODULE name (`Int`/`List`/`Name`/…) — the exact ones the `9f326a2d` skip drops so a
    /// bare `Int` keeps resolving to the width constructor everywhere it means the width type (a `(Int W)`
    /// annotation reduction, `Int64`'s synthesized `(Int 64)`). Kept SEPARATE (not in the main bare index)
    /// so those synthesized/type-position uses are unaffected: `resolve_name` consults THIS map ONLY in
    /// application-HEAD position on a genuine USER node — the same head-position/user-node discriminator the
    /// same-name-newtype ctor rule uses — where a bare `(Int 42)` is a value CONSTRUCT of the local variant,
    /// not a width-type reduction. This realizes the operator ruling that a user variant name SHADOWS the
    /// colliding prelude name in construct position too (`prelude-and-resolution.md`: binding is lexical, a
    /// program-defined name shadows the built-in alias). First-declared wins, matching `variant_ctor_index`.
    prelude_colliding_variant_ctor_index: crate::fxhash::FxHashMap<String, StructId>,

    /// Every node that lies inside a TYPE-EXPRESSION subtree — a variant payload (`(List Ast)` in `(type
    /// Ast … (List (List Ast)))`), an annotation type slot (`(: x (List Int64))`'s `(List Int64)`), or an
    /// effect-op arrow (`(op f (-> (List a) Bool))`). The construct-position shadow (step 3d in
    /// `resolve_name`) fires only in genuine VALUE-construct head position, so it consults this set and
    /// FALLS THROUGH (to the prelude, as before) for a head inside a type expression: there `(List Ast)`
    /// means the List TYPE constructor, not a value construct of a `List` variant — diverting it would turn
    /// a self-referential AST sum's payload into a bogus variant application (CDZ0201). A bare type ATOM
    /// (`Int64` in `(: x Int64)`) is already spared by the head-position (`child_ix == 0`) gate, so this
    /// set only needs to cover type-expression APPLICATION heads; marking whole subtrees is a safe superset.
    /// Built once at load. Missing a root only RE-OVER-REJECTS that type expression (never a miscompile),
    /// matching the pre-fix behavior — the shadow is additive over a strictly value-position surface.
    type_expr_nodes: crate::fxhash::FxHashSet<StructId>,

    /// For each user-sum TYPE NAME, its synthesized RECORD occurrence (first-declared wins). Built once at
    /// load from `type_decls[].{name,synth}`. `type_decl_by_name` consults it for a name reference that
    /// falls through scope + defs in `resolve_name` (step 3) — and `resolve_name` reaches step 3 for MANY
    /// references (a bare variant, a type annotation, any non-local non-def name), so the old linear
    /// `type_decls.iter().find(|t| t.name == name)` was O(types) per resolution → O(N²) for a program with
    /// N sum types (measured: 3200 sum types ≈ 590ms, `resolve_name`+`memcmp` ~50% self). O(1) lookup;
    /// first-wins matches the old scan. A pure accelerator over `type_decls`, never a source of truth.
    type_decl_index: crate::fxhash::FxHashMap<String, StructId>,

    /// SAME-NAME-NEWTYPE name → its ctor occurrence — a load-time index for [`same_name_newtype_ctor`]
    /// (`(type UserId (UserId Int64))`: name `UserId` → its single variant's ctor). That lookup was a
    /// linear `type_decls.iter().find(|t| t.name == name && one-variant && variant.name == name)`, and the
    /// linked-package resolve path (`resolve_name`'s file-scoped type/ctor head-position steps) probes it
    /// per type-name reference → O(N²) for a package with N types (measured: N libraries each imported +
    /// used, `resolve_name`+`memcmp` ~29% self at N=1600). O(1) lookup; first-declared wins, matching the
    /// scan's first-hit. A pure accelerator over `type_decls`.
    same_name_newtype_ctor_index: crate::fxhash::FxHashMap<String, StructId>,

    /// The subset of same-name-ctor names whose owning sum is MONOMORPHIC (no type params). `resolve_name`
    /// consults this to fire the head-position ctor rule on a SYNTH node too (a β-copied `(Meters a)`
    /// inlined at a call site), which the `is_user_node` gate alone wrongly left the TYPE — safe only for a
    /// monomorphic sum (a generic one has a `sum_applied` synth type-expr that must stay the type). See
    /// [`Db::same_name_monomorphic_ctor`].
    same_name_monomorphic_ctor_index: crate::fxhash::FxHashSet<String>,

    /// For each sum's DECLARATION OCCURRENCE, its index in [`type_decls`] — the O(1) reverse of the nominal
    /// identity a `Ty::Sum { decl }` carries, backing [`type_decl_by_occ`]. That was a linear
    /// `type_decls.iter().find(|t| t.occ == occ)`; the Rust backend's recursive-sum boxing walk calls it
    /// once per sum-graph node per query, so for N mutually-recursive sums it compounded an O(N²) reach
    /// walk into O(N³) (N=3200 cycle = 39s on the rust target). Built once at load ([`type_decls`] is not
    /// mutated after construction). `type_decl_by_occ` reads it, falling back to the scan for an occ minted
    /// after load (there is none today, but the fallback keeps it total).
    type_decl_by_occ_index: crate::fxhash::FxHashMap<StructId, usize>,

    /// For each EFFECT NAME, its synthesized record occurrence (first-declared wins) — the effect analogue
    /// of [`type_decl_index`], for `effect_decl_by_name`'s step-3b resolve lookup (was a linear
    /// `effect_decls.iter().find`). Built once at load.
    effect_decl_index: crate::fxhash::FxHashMap<String, StructId>,

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

    /// Per-MATCH-ARM BINDER-name OVER-APPROXIMATION: `arm_form_occ → the set of the name atoms in the arm's
    /// PATTERN region (all children except the body) that sit in a BINDER position` — i.e. every name
    /// EXCEPT the HEAD (child 0) of each list form (see [`build_arm_pattern_names`], built once at load).
    ///
    /// A match arm's `binder_in` cases (`resolve::binder_in` Cases 5/5g/6/6l/6r/6mr/6g/…) all bind a name
    /// written into the arm's pattern in a NON-HEAD position (a whole-scrutinee bare binder, or a variant/
    /// tuple/list/record/map/bin payload binder — a ctor PAYLOAD, tuple/list ELEMENT, record field VALUE,
    /// seg-binder, map VALUE, or rest binder). A pattern HEAD (`Some` in `(Some x)`; `tuple`/`list`/
    /// `record`/`bin`/`map`/`guard`; a record field NAME; a seg TYPE; a map KEY) is NEVER a binder, so
    /// excluding heads keeps a safe SUPERSET of the arm's binders while dropping constructor/keyword names.
    /// A lexical-scope walk ascends EVERY enclosing binder, and in a DEEPLY-NESTED match (recursive-AST-walk
    /// shape) the vast majority of the ~20 per-arm case probes conclude "binds nothing" — a reference to a
    /// GLOBAL/prelude/CTOR name (`Some`, `None`, `+`) is bound by no arm, so it ran the whole
    /// `match_arm_*_binds`/`guard_cond_*_binds` cascade at each of O(depth) enclosing arms before falling
    /// through to the prelude (measured: the arm cascade was ~66% of resolve self-time, O(depth)-per-
    /// reference on a depth-N nested match). This set lets `binder_in` fast-REJECT such a hop in O(1): if
    /// `name` is not in the arm's binder-name set, no arm case can bind it, so return `None` without the
    /// cascade. Excluding heads is what extends the fast-reject to CONSTRUCTOR-name references (`Some`/
    /// `None` appear ONLY as pattern heads, so an all-atoms set would keep running the cascade for them —
    /// leaving ctor-name references O(N²)). OVER-approximation is safe by construction — a name genuinely
    /// bound is always in the set (a binder is a non-head pattern atom), so a real binding is never skipped;
    /// a spurious extra name only forgoes the skip (runs the cascade, which returns the same result). Load-
    /// time arm patterns are immutable (arena lists are immutable; desugars build fresh nodes with NEW ids
    /// that are absent from this map → they take the cascade), so the load-time set stays a valid over-
    /// approximation for the arm ids it holds.
    arm_pattern_names: crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashSet<String>>,

    /// Per-LET-BINDINGS-LIST binder index: `(bindings_list_occ, name) → the ASCENDING positions of that
    /// name's bare bindings + each one's value occurrence`. The `let` analog of [`scope_binders`].
    ///
    /// `last_binder_named` answered "the last binding of `name` visible before position `end`?" by a
    /// REVERSE SCAN of the in-scope pairs. That scan runs to completion for a NEGATIVE lookup (a reference
    /// to a prelude/outer name — `+`, `Int64` — that the let does not bind) and for a binder's own
    /// shadow-check, so a `(let ((v0 …) … (vN …)) …)` where each initializer references an earlier binder
    /// was O(N) per reference × O(N) references = O(N²) (a wide accumulation `let` — realistic code — was
    /// ~1440 avg scan iters at N=3200). This index makes each lookup O(log N) (a `partition_point` for the
    /// last position < `end`) and a negative lookup O(1).
    ///
    /// Built ONLY for a bindings-list whose every pair is a bare-name (or `(: name T)` annotated-bare)
    /// binding — the case whose linear branch returns a plain `Ref { value }`. A list containing a
    /// DESTRUCTURING pattern binding (`((tuple a b) V)`/`((Ctor n) V)`, which resolves to a `SumPayload`
    /// path) is ABSENT from this map, so `last_binder_named` falls back to the exact linear walk for it —
    /// the fast path never has to replicate the pattern-binder enumeration, and its verdict is byte-
    /// identical (last-wins among bare bindings before `end`). See [`build_let_binder_index`].
    let_binder_index:
        crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>>>,

    /// Per-DO-BLOCK / per-MODULE declaration index: `scope_form_occ → (declared_name → the ASCENDING
    /// positions of the `def`/`module` forms that declare it, each with the declaring FORM's occurrence)`.
    /// Keyed by a `(do …)` OR a `(module …)` occurrence (both hold a name-resolved declaration sequence).
    /// The do-block/module analog of [`let_binder_index`] / [`scope_binders`].
    ///
    /// `binder_in`'s do-block case answered "does an earlier form declare `name`? and (for mutual
    /// recursion) does ANY form declare it as a function?" by SCANNING every do-form: a reverse scan for
    /// the last sequential binding (`do_def_binds`/module per form), then — on any miss — an UNCONDITIONAL
    /// forward scan of ALL forms for a `Lambda`. Both run to completion on a NEGATIVE lookup (a reference
    /// to a prelude/outer name — `+`, `unit`, a member access's own head — that the block does not
    /// declare), so a `(do (def f0 …) … (def fN …) <N uses>)` or a wide do-local `(module …)` accessed
    /// from N sites was O(forms) per reference × O(refs) = O(N²) (the `let`-binder trap of fix-17, here in
    /// the do-scope). This index lists ONLY the forms declaring each name, so both scans visit those few
    /// (usually one) instead of all N — a `partition_point` for the last position < the reference's, and
    /// an O(1) "is any of them a function def?" for the mutual-recursion fallback. A name the block does
    /// not declare is absent → an O(1) negative. The value stays the do-form occurrence (not a resolved
    /// binder) so `do_def_binds` computes the exact same `Ref`/`Lambda` it did before — byte-identical.
    do_binder_index:
        crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>>>,

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

    /// The load-time boundary of [`scope_skip`]: the arena length when the index was built at load. A node
    /// id BELOW this is a load-time node whose skip entry is GENUINE (`build_scope_skip` computed it). A
    /// node id at/above this is a post-load SYNTHESIZED node — its skip entry is genuine ONLY if it was
    /// explicitly seeded (recorded in [`scope_skip_seeded`]); otherwise a `None` entry there is merely the
    /// `resize`-default of some UNRELATED `extend_scope_skip_*` call that grew the vector past it, NOT a
    /// computed "resolves to prelude". Conflating the two is a poison: a synth node swept into the vector by
    /// a later resize would take the skip FAST-PATH (bottoming at its default-`None` → prelude → a spurious
    /// unbound), when it should fall back to the exhaustive parent walk. See [`scope_skip_covers`].
    scope_skip_load_boundary: usize,

    /// Post-load synth node ids whose [`scope_skip`] entry was EXPLICITLY seeded (by an `extend_scope_skip_*`
    /// call), so the fast-path is safe for them. A synth node NOT in this set — even if the vector was
    /// resized past it by an unrelated seeding — is treated as UNCOVERED (exhaustive walk), never as a
    /// computed prelude-fallback. See [`scope_skip_load_boundary`].
    scope_skip_seeded: crate::fxhash::FxHashSet<StructId>,

    /// Node ids FORCED to fall back to the exhaustive parent walk (never the [`scope_skip`] fast-path), even
    /// though they are LOAD-TIME nodes the fast-path would otherwise cover. A load-time name occurrence
    /// RE-PARENTED into a synthesized structure (e.g. the escaped-closure recovery β-inlines a helper call
    /// inside a `let` binding-init and rebuilds the `let`, re-parenting the load-time body reference under the
    /// fresh `let`) keeps its LOAD-TIME skip entry — which still points at its ORIGINAL enclosing scope —
    /// because `reparent`/`push_list` do not recompute `scope_skip`. Taking the fast-path then resolves the
    /// reference against its OLD binder (a stale init occurrence) instead of the rebuilt one, so a later
    /// hygiene rename misses it and the reference is left dangling → a false CDZ0101 (breaker iso-b). Marking
    /// such a node here forces the STRUCTURAL walk, which follows the CURRENT parent chain to the rebuilt
    /// scope — the ground truth. Always safe (the walk is exhaustive-correct; the fast-path is only an
    /// optimization), just slower for the marked nodes; applied narrowly (the recovery's small inlined body).
    scope_skip_force_uncovered: crate::fxhash::FxHashSet<StructId>,

    /// The set of nested-module SYNTHESIZED RECORD ids — each is a binding scope (its members are mutually
    /// visible), so [`is_binding_candidate`] treats it as a candidate. Retained from load so the
    /// CANDIDATE-AWARE scope-skip extension (`extend_scope_skip_into_subtree`) can re-run
    /// `is_binding_candidate` on a synthesized subtree's nodes (a desugar that builds a DEEP chain
    /// CONTAINING a binding form — a `(guard …)` — must land inner references on that form, not hop past
    /// it). An O(1) membership probe.
    module_records: crate::fxhash::FxHashSet<StructId>,

    /// The prelude — the one map of built-in bindings, installed ONCE at load as ordinary AST nodes
    /// (a built-in module is just a record; see `crate::prelude`). Maps a built-in name to the arena
    /// occurrence it binds to. `resolve` consults it after the lexical scope, so a program binding
    /// shadows a built-in by the ordinary lookup precedence.
    pub prelude: BTreeMap<String, StructId>,

    /// The FAMILY-OF-MEASURE registry — maps each named family unit to its `(dimension, scale
    /// numerator, scale denominator)`, where the DIMENSION is a list of `(base-name, exponent)` pairs (an
    /// exponent map). An ATOMIC-dimension unit has a one-entry dimension: `"foot"` →
    /// `([("meter", 1)], 381, 1250)`, `"minute"` → `([("second", 1)], 60, 1)`. A DERIVED-dimension unit
    /// has a multi-entry one: `"mbps"` (megabit/second) → `([("byte", 1), ("second", -1)], 1_000_000, 8)`
    /// — a unit of the `information/time` dimension whose scale to the reference `byte/second` is a
    /// megabit over 8 bits-per-byte (`units-of-measure.md` §A Dimension Groups Interconvertible Units).
    /// `(Unit.of #"mbps")` consults this to build that dimension (via `Unit::base`/`mul`/`pow`) scaled by
    /// `num/den`. The vocabulary is prelude DATA (`prelude::unit_families`), so the layer fixes the
    /// MECHANISM, not a privileged list; scales are machine-int metadata, so a family unit auto-converts
    /// over Float/Int with no arbitrary-precision arithmetic.
    pub unit_families: BTreeMap<String, crate::prelude::UnitConversion>,

    /// USER-DECLARED family units — the `(Unit.define #"name" base-unit-expr num den)` forms a program
    /// writes, scanned at load: `(name, the base-unit-expression's occurrence, scale-num, scale-den)`.
    /// `(Unit.define #"furlong" (Unit.of #"foot") 660 1)` declares a furlong = 660 feet. `Unit.of` and
    /// the conflict check consult these (reducing the base-unit occurrence via `eval::unit_of` on demand,
    /// which needs the built `Db` — hence a raw-occurrence store, not a reduced one). A name declared
    /// with a conversion conflicting with the built-in table or another declaration is `CDZ0502`
    /// (`units-of-measure.md` §A Named Unit's Conversion Is Unique), checked in the passes. This is the
    /// "a program MAY declare its own families" surface (`options/units-of-measure/`).
    pub unit_defines: Vec<(String, StructId, i128, i128)>,

    /// PACKAGE LINKAGE — `Some` only for a multi-file package (`DESIGN-package-linking.md`), `None` for
    /// the single-file compile (whose namespace is flat, byte-identical to the pre-linking compiler).
    /// When present it makes name resolution FILE-SCOPED: a bare name in file `f` resolves against
    /// `f`'s own top-level defs and `f`'s imports, never a sibling file's defs (`resolve::resolve_name`
    /// step 2 reads `file_scope` instead of the global `def_by_name` when this is `Some`).
    pub(crate) file_scope: Option<FileScopeTable>,

    /// The evaluator's current β-reduction nesting depth. β-reduction inlines a callee's body; a
    /// TERMINATING fold bottoms out at a bounded depth, while a RECURSIVE function inlines without end.
    /// So a bounded depth is the sound guard: past [`REDUCE_DEPTH_LIMIT`] the evaluator declines rather
    /// than diverging (`reference-compiler.md` §The Evaluator Bounds Its Own Reduction And Declines
    /// Rather Than Diverges). (A body-on-the-stack set is NOT a sound recursion signal — `(f (f v))`
    /// legitimately nests the same body twice while both calls terminate; only the depth distinguishes
    /// a terminating nest from an unbounded one.)
    pub(crate) reduce_depth: u32,

    /// The running COUNT of nodes β-reduction has synthesized this compile — the WORK budget the depth
    /// counter cannot provide. `reduce_depth` bounds how DEEPLY reductions nest, but a term can grow
    /// EXPONENTIALLY in SIZE across a bounded depth: a self-applying lambda `(fn (v) (v (v v)))` bound to
    /// `x` and applied `(x x)` roughly DOUBLES its application count each reduction, so even 32 bounded
    /// levels synthesize ~2^32 nodes — the depth guard is satisfied while the compiler does unbounded
    /// work and appears to hang (the `cdz-smith` timeout). A monotonic node budget over the whole compile
    /// is the sound second guard: past [`REDUCE_NODE_BUDGET`] the evaluator declines the reduction (a
    /// non-normalizing / explosively-growing term) rather than diverging, exactly as the depth guard
    /// declines a too-deep one. Reset to 0 per `Db` (a fresh compile); never decremented (it measures
    /// total synthesis work, not current depth).
    pub(crate) reduce_nodes: u64,

    /// PER-DEF-BODY count of STRUCTURAL reductions ([`enter_reduction_structural`] entries) — RESET to 0 at
    /// the top of each def-body walk in `collect_faults` (NOT per `Db`: a program-level cumulative count
    /// would re-introduce the Option.None carve-out regression the structural path exists to avoid, since a
    /// real multi-module compile legitimately does ~800k structural reductions cumulatively). Bounds the
    /// self-app structural-explosion HANG per body against [`STRUCTURAL_REDUCTION_BUDGET`].
    pub(crate) structural_reductions: u64,

    /// SIDE-CHANNEL for the effect fold's ILL-TYPED-COMPOSITION decline (CDZ0203). `reduce_handle` returns
    /// `Option` (a bare decline), so when its pure-one-hole fold produces a folded term that FAILS the type
    /// checker — an arm consuming the continuation result at a type the handle body cannot supply, e.g.
    /// `(+ 1 (resume …))` over a Bool-typed body folds to `(+ 1 (< 10 5))` — it records the type fault HERE
    /// before declining, so the emit path (`lower/compute.rs`) can surface the genuine CDZ0203 `TypeMismatch`
    /// instead of the generic CDZ0900 not-reducible decline (a `resume` types as `Ty::Any`, so this
    /// ill-typedness is only visible POST-fold, never at inference). The caller CLEARS it before invoking
    /// `reduce_handle` and `.take()`s it only on a `None` result, so a nested fold's fault never leaks.
    pub(crate) fold_type_fault: Option<crate::diag::Reject>,

    /// The current RECURSIVE-DESCENT depth across the demand queries (`type_of`, `collect`, `core_of`)
    /// — the recursive-descent backstop. Bumped on entering a query's recursion and restored on exit;
    /// past [`DESCENT_DEPTH_LIMIT`] the query declines instead of recursing, so pathologically deep
    /// input (or an unproductive self-recursion) yields a resource-limit decline rather than a native
    /// stack overflow. Shared across the queries, which interleave (`collect` calls `type_of`), so the
    /// counter reflects true stack depth; it returns to 0 between top-level demands. See
    /// [`DESCENT_DEPTH_LIMIT`].
    pub(crate) descent_depth: u32,

    /// The current recursive-descent depth of a LAYOUT core-tree WALK (`collect_call_callees`,
    /// `collect_closure_codes`) — a SEPARATE counter from [`descent_depth`], deliberately NOT shared. These
    /// walks call [`crate::lower::core_of`] at each node (which uses `descent_depth` for its OWN recursion),
    /// so folding the walk depth into `descent_depth` would inflate `core_of`'s view and spuriously decline
    /// a valid program whose core is moderately deep. This counter bounds ONLY the walk's own native
    /// recursion: a non-normalizing self-application in a sum-constructor payload materializes a
    /// `Core::SumNew` chain the walk would otherwise descend until the native stack overflows (the walk
    /// lazily β-reduces one payload level per `core_of`, unbounded by the reduction-DEPTH guard). Past
    /// [`WALK_DEPTH_LIMIT`] the walk stops descending — the program is rejected by `collect_faults` anyway,
    /// so a clipped reachable set changes no accepted program. Bumped/restored around each walk recursion.
    pub(crate) walk_depth: u32,

    /// VISITED-SETS for the three SET-producing LAYOUT walks (`collect_call_callees`,
    /// `collect_closure_codes`, `collect_closure_call_sigs`). Each walk follows [`crate::lower::core_of`],
    /// which resolves a `Ref` to its target's body, so a compound value used at several sub-positions is a
    /// SHARED core DAG the naive recursion re-walks as a TREE — `O(K^depth)` on a wide fan-out (the CMB1
    /// binomial-walker compile-hang). Each walk's OUTPUT is a SET whose membership is a pure function of the
    /// node (a callee reached via two DAG edges is the same callee; a closure code / call-sig likewise —
    /// multiplicity is irrelevant, and every downstream consumer dedups), so skipping an already-walked node
    /// changes no result. This is the same soundness argument [`reached_visited`] relies on for the
    /// reached-poison walk. Unlike that walk these need NO clip-flag: the layout walks' `WALK_DEPTH_LIMIT`
    /// clip is already documented accepted-program-neutral (a clipped program is rejected by `collect_faults`
    /// anyway), so recording a depth-clipped node cannot suppress a fault in an ACCEPTED program. Each set is
    /// CLEARED at its walk's top-level entry (`walk_depth == 0`): the walks never cross-nest and `core_of`
    /// never re-enters them, so `walk_depth == 0` is exactly "no walk of this kind is active" — a fresh
    /// per-entry set, which is required because `collect_call_callees` runs PER-DEF (a persistent set would
    /// give a later def an incomplete callee set → a dropped emit / miscompile).
    pub(crate) callee_visited: crate::fxhash::FxHashSet<StructId>,
    pub(crate) closure_code_visited: crate::fxhash::FxHashSet<StructId>,
    pub(crate) closure_sig_visited: crate::fxhash::FxHashSet<StructId>,
    /// Sharing-aware visited-set for the host-import walk ([`crate::backend::wasm::host::collect_host_imports`]),
    /// same class + soundness as [`Self::callee_visited`]: the host-import SET is presence-only (the walk
    /// self-dedups by (effect,op)), so skipping an already-walked shared node changes no output while
    /// collapsing the DAG re-descent. Cleared at the walk's top-level entry (`walk_depth == 0`).
    pub(crate) host_import_visited: crate::fxhash::FxHashSet<StructId>,
    /// Sharing-aware visited-set for the host-arg-string walk
    /// ([`crate::backend::wasm::collect_host_arg_strings`]) — same class: the consumer lays each DISTINCT
    /// string once (presence), so skipping an already-walked shared node changes no laid-string set.
    pub(crate) host_arg_string_visited: crate::fxhash::FxHashSet<StructId>,

    /// Set true when a `collect` subtree hit a depth/reduction LIMIT (the descent-depth backstop, or a
    /// reduction that could not proceed because `enter_reduction` was exhausted). A limit-clipped walk is
    /// PARTIAL — it declined early rather than seeing the node's true faults — so its result must NOT be
    /// cached in [`collect_cache`] (at a shallower entry depth the same node would collect fully). The
    /// memo wrapper reads-and-clears this around each subtree: if it was set, the result is not cached.
    pub(crate) collect_limited: bool,

    /// Nodes whose `collect_reached_poisons` subtree has ALREADY been walked to completion (unclipped) in
    /// the CURRENT `collect_faults` call — a visited-set that keeps the reached-poison walk from
    /// re-descending a shared core DAG as a tree. The walk follows `core_of`, which resolves a `Ref` to
    /// its target's body, so a value used in two operand positions (`(* a a)` — repeated squaring over
    /// NON-folding `BigInt` operands, where each `(* a_i a_i)` reaches `a_{i-1}`'s shared operand nodes
    /// from BOTH sides) makes the walk O(2^depth) without this. A poison's contribution is a pure function
    /// of its node (its origin is its own id, path-independent) and `dedup_faults` collapses duplicates, so
    /// skipping an already-walked node changes no reported fault. CLEARED at each `collect_faults` entry
    /// (the walk's faults accumulate into that call's vec; a later call must re-walk from scratch). A node
    /// is inserted only when its subtree walked UNCLIPPED — a depth-clipped subtree is partial, so a later
    /// shallower path must be free to walk it fully (the same discipline [`collect_limited`] gives
    /// [`collect_cache`]); [`reached_clipped`] carries that signal up.
    pub(crate) reached_visited: crate::fxhash::FxHashSet<StructId>,

    /// Set true while a `collect_reached_poisons` subtree hit the [`DESCENT_DEPTH_LIMIT`] backstop — the
    /// signal that the subtree is PARTIAL, so its root must NOT be recorded in [`reached_visited`]. Reset
    /// to `false` before each node's recursion and read after, so a node learns whether ANY descendant
    /// clipped; the outer value is OR-ed back in so the clip propagates to ancestors.
    pub(crate) reached_clipped: bool,

    /// SYNTHESIZED-NODE → SOURCE-NODE provenance for β-reduction NAME COPIES. When `copy_structural`
    /// copies a name atom (a fresh occurrence past `user_node_count`, so `is_user_node` is false and it
    /// has no span), it records here the ORIGINAL source occurrence it was copied from. This lets a
    /// diagnostic anchored at the copy be RELOCATED to the real source node (`source_of_synth`) rather
    /// than un-anchored: a name that re-resolves UNBOUND in an inlined copy (a mutual-recursion cycle
    /// member's param that lost its binder scope in the spliced body — the whole-program CDZ0101 that,
    /// unlike the per-file `cdz check` path, carries no location) then points at the source reference the
    /// author wrote. Populated only for name copies (the constant-atom / list arms share or re-push
    /// without a source name to attribute); a copy-of-a-copy chains through so the anchor lands on the
    /// original user occurrence. Purely a location aid — it never changes WHETHER a fault is reported.
    pub(crate) synth_name_origin: crate::fxhash::FxHashMap<StructId, StructId>,

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

    /// Memo of `lower::subtree_reaches_host_call` — whether the subtree at a node reaches a `Core::HostCall`
    /// (an observable effect that DCE must keep). A pure function of the fixed AST/core structure, keyed by
    /// node `StructId`. WITHOUT it the predicate re-walks a node's WHOLE subtree calling `core_of` per node,
    /// and it is called from TWO hot passes over overlapping subtrees — the `do`/`let` lowering
    /// (per do-block) and `collect_discarded_value_warnings` (per non-final statement) — so a `do` block's
    /// statement subtrees were re-walked repeatedly. On the real corpus workload this pair was ~45% of
    /// compile time (`subtree_reaches_host_call` + its per-node `core_of`). The memo makes each node's
    /// verdict O(1) after first computation. `None` until computed.
    pub(crate) reaches_host_call: crate::fxhash::FxHashMap<StructId, bool>,
    /// Memo of `select::reclaim::binding_escapes_dup_aware` (the emit-side Perceus escape query), keyed by
    /// the FULL context the verdict depends on — `(node, EscapeTarget, tail_borrowed)` — and used ONLY when
    /// `dup_sites` is `None` (the sumpayload/cont-escape path; the `Some` drop-site path neither reads nor
    /// writes it, so no cross-contamination). The escape verdict is a pure function of that key + the
    /// build-once-immutable Core graph (v-memory-safety sign-off), so a compile-lifetime memo is sound and
    /// needs no clearing. This LINEARIZES the DAG-as-tree escape re-walk that made the DbState-handler-heavy
    /// modules (db-query-diff / db-query-perfield) `cdz test` compile super-linearly (front-3). NO
    /// in_progress/tainted guard — the walk is acyclic by spec (a recursive def refers to itself via a static
    /// code ref, never a heap-value cycle; the `Core::Call` arm recurses only into args, not the callee body)
    /// and the memo writes only AFTER a call returns, so a (spec-impossible) cycle node never returns → never
    /// caches an artifact (v-mem: adding an in_progress that returns for a back-edge is what WOULD create the
    /// cycle-artifact risk, so it is deliberately omitted).
    pub(crate) escape_verdict_memo: crate::fxhash::FxHashMap<
        (StructId, crate::backend::wasm::select::EscapeTarget, bool),
        bool,
    >,
    /// Memo of `select::reclaim::collect_consuming_payload_sites_expr` — each subtree's OWN complete set of
    /// consuming compound-leaf scrutinee-child sites, keyed by `(node, scrut, consuming)`. The contribution
    /// is a pure function of that key + the build-once-immutable graph (payload_proj_chain_roots/get_op/
    /// core_of are pure; `consuming` is the borrow-vs-consume position flag; NO sibling `live_after` here and
    /// NO read of the shared `out`, unlike mark_binder_dups — v-memory-safety sign-off). LINEARIZES the ~643×
    /// DAG-as-tree re-walk (collect_consuming_payload_sites_cont calls the expr-walker on the SAME shared
    /// scrutinee-expr per continuation arm). Compile-lifetime (immutable graph, no clearing); NO in_progress/
    /// tainted (acyclic by spec + memo writes post-return). Cached value is the node's OWN contribution (a
    /// FRESH accumulator, NOT a delta vs shared `out`) — spliced via `out.extend` on a hit.
    pub(crate) payload_sites_memo:
        crate::fxhash::FxHashMap<(StructId, StructId, bool), Vec<StructId>>,
    /// Memo of `select::reclaim::collect_captured_occurrences` — each subtree's OWN `Core::Captured`
    /// occurrences grouped by capture-slot index, in `core_child_ids` DFS (pre-)order. Keyed by `id` ALONE:
    /// the collector's only context is `id` (it branches solely on `Core::Captured`, no scrut/consuming/
    /// tail_borrowed) — the simplest key in the reclaim-memo family (v-memory-safety sign-off). LINEARIZES
    /// the ~570× DAG-as-tree re-walk (the collector runs per lifted-body → shared subtrees re-walked per
    /// closure). Compile-lifetime (immutable graph); NO in_progress/tainted (acyclic + writes post-return).
    /// MULTIPLICITY: the value is the node's OWN contribution (a FRESH accumulator, NOT a delta vs the shared
    /// by_index); MERGE = APPEND on EVERY hit (never dedup, never once-per-distinct-id) so a shared subtree
    /// reached via N paths yields N appends = the SAME per-visit multiplicity the re-walk produced; occurrence
    /// order is DFS to match the re-walk.
    pub(crate) captured_occ_memo:
        crate::fxhash::FxHashMap<StructId, std::collections::HashMap<usize, Vec<StructId>>>,

    /// Memo of the wasm-CSE `is_cse_shareable(id)` predicate (`backend::wasm::select`), keyed by node id
    /// ALONE — the SIMPLEST key in the emit-memo family (like `captured_occ_memo`). The predicate is a PURE
    /// function of the node's immutable Core subtree (each arm recurses on child ids via `core_of`, no
    /// mutable/thread-local/flow context, no `dup_sites`/`tail_borrowed`), so its verdict for a fixed id is a
    /// stable fact of the build-once graph. The straight-line CSE driver queries it per candidate node, so on
    /// a deeply-nested expression the recursive subtree walk is re-run for every enclosing node → O(N²)+
    /// (a `cdz compile` probe on a nested-match/arith chain showed ~O(N^2.6) emit, `is_cse_shareable` 70%
    /// self+inclusive). Caching linearizes it. Compile-lifetime; NO in_progress/tainted needed — the walk is
    /// ACYCLIC (it recurses only through pure structural expression children; a `Core::Call`/heap-construct is
    /// a leaf that returns `false`, never descending into a callee body), so a memo write post-return can
    /// never observe a cycle. Byte-neutral: the SAME bool is computed, just once, so every CSE decision is
    /// identical → identical emitted wasm.
    pub(crate) is_cse_shareable_memo: crate::fxhash::FxHashMap<StructId, bool>,

    /// Memo of "does this compound type have a free var?" keyed by the payload's shared `Rc` address — for
    /// the `infer::type_of` memoization guard (`!t.has_free_var()`), which runs on EVERY node's solved type.
    /// A wide `Ty::Record`/`Ty::Tuple` (an N-field record) referenced from N nodes had the guard walk its
    /// whole O(N) payload once per node → O(N²). The payload is IMMUTABLE and its `Rc` is SHARED across all
    /// those nodes (a memoized `typeval_of` / a solved param type hands back the same `Rc`), so the verdict
    /// caches by the `Rc`'s `as_ptr` address (the fix-50 key). Only `true`/ground `false` verdicts are
    /// definite facts of the fixed payload, so caching is sound; the map lives on the per-compile `Db`.
    pub(crate) ty_has_free_var: crate::fxhash::FxHashMap<usize, bool>,

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

    /// Memo of a RECORD TYPE's `field-name → sorted-position` index (`eval::runtime_member_index`), keyed
    /// by the identity of the type's shared `Rc<BTreeMap<Symbol, Ty>>` (its `Rc::as_ptr` address). A field
    /// read on a RUNTIME record `(. r f)` lowers to a `Core::Proj` at `f`'s sorted slot, which was found by
    /// a LINEAR `fields.keys().position(|k| k == key)` scan — O(fields) per projection, so a wide record
    /// projected field-by-field was O(N²) (a 3200-field record: 644ms, ~O(N²) growth). The field order is a
    /// pure function of the (immutable) record type, and every projection of the same record shares its type
    /// `Rc`, so building the full name→index map ONCE per record type and reading it O(1) removes the
    /// per-projection scan. The stored `guard` `Rc` keeps that allocation alive, so its `as_ptr` address
    /// cannot be reused by a different `BTreeMap` while the entry is cached (ABA safety).
    pub(crate) record_field_index: crate::fxhash::FxHashMap<usize, RecordFieldIndex>,

    /// Memo of a LIST PATTERN's leading-element BINDER index (`resolve::list_pattern_binders`), keyed by
    /// the pattern's `(list …)` occurrence. `resolve::find_leading_binder_in_list_pattern` answered "does
    /// this list pattern bind `name`, and at what access path?" by re-scanning the pattern's LEADING
    /// element positions from 0 on EVERY reference — a positive lookup to the binder's position, a NEGATIVE
    /// one (a prelude/outer name — `+`, `Int64`, a callee — the pattern does not bind) over the WHOLE
    /// pattern — so a wide `(match xs ((list a0 … aN .. r) body) …)` destructure whose body references each
    /// binder (or the synth `(list __le0 … __leN .. r)` the refutable-literal-element desugar builds, each
    /// `__le` referenced by its guard test) was O(N) per reference × O(N) references = O(N²) (measured: the
    /// `flat`-body shape N=1600 ~140ms, ~4×/dbl, `find_leading_binder_in_list_pattern` ~37% self + its
    /// per-element path `Vec` alloc ~45% of malloc). The pattern's binder set is a pure function of the
    /// (immutable, append-only) arena, so building the whole `name → (path, heads)` map ONCE per pattern
    /// and reading it O(1) removes the per-reference scan. Built lazily via a `RefCell` (resolution runs
    /// behind `&Db`; the compiler is single-threaded / `!Send`, so interior mutation here is sound). Only a
    /// SIMPLE pattern — every leading element a bare-name/`_`/`..` binder, no nested tuple/variant/list
    /// element — is indexed (`Some(map)`); a pattern with a nested element caches `None` and the caller
    /// falls back to the exact linear descent (whose verdict is byte-identical), so the fast index never
    /// has to replicate the nested-payload path enumeration. The two demonstrated explosions are both
    /// all-bare-leading, so the fast path covers them; a nested-element wide list arm declines at lowering
    /// (`more than one refutable constructor element`) long before it could matter.
    pub(crate) simple_list_binders: std::cell::RefCell<
        crate::fxhash::FxHashMap<StructId, Option<std::rc::Rc<SimpleListBinders>>>,
    >,

    /// Memo of a `let` region's binding USE-FACTS (`lower::BindingUses`), keyed by the `(let …)` node whose
    /// region (all its initializers + body) was walked to build it. `lower::lower_let` decides per binding
    /// keep-vs-copy-propagate from these facts, collected in ONE walk of the region (fix-44). For a DEEP
    /// nested chain `(let ((v0 …)) (let ((v1 …)) … body))`, each of the N levels' `lower_let` re-walked its
    /// body — the entire deeper O(N−k) chain — so Σ = O(N²) (the deep-nested TWIN of fix-44's wide case; the
    /// wide case fused a single let's per-binding walks, this fuses the per-LEVEL body re-walks). `let*`
    /// scoping makes an OUTER let's whole-region facts EXACT for every nested binding too — a binding `v_k`'s
    /// references live only from its own init onward (an earlier init cannot name a later binding), a subset
    /// of the outer region — so a nested `lower_let` reuses the nearest enclosing cached region's map instead
    /// of re-collecting. The `Rc` is shared across all levels of one nest; the map only grows (an entry, once
    /// built, is stable), so interior mutation via the `RefCell` is sound (single-threaded / `!Send`).
    pub(crate) let_region_uses: std::cell::RefCell<
        crate::fxhash::FxHashMap<StructId, std::rc::Rc<crate::lower::BindingUses>>,
    >,

    /// Memo for the wasm backend's `mutual_loop_group(self_def)` — the tail-recursive SCC a def compiles
    /// its shared loop over. `select_function_of` computes it for EVERY def, and the computation is a
    /// double BFS over tail-callees (forward-reach + reach-back-to-self per member), so N mutually
    /// tail-recursive same-signature defs cost O(N²) EACH → O(N³) over the group (measured: 200 mutual
    /// defs = 687ms, ~O(N³), with O(N²)-size wasm). The group is a pure function of the call graph +
    /// signatures (fixed), keyed by the def's own index (the ordering is `self_def`-first, so it differs
    /// per member — cache each member's own view). Caching collapses the per-def recompute; a def not in
    /// any loop caches an empty vec. Keyed by def index (a `usize` into `defs`).
    pub(crate) mutual_loop_cache: crate::fxhash::FxHashMap<usize, Vec<usize>>,

    /// Memo of a lambda APPLICATION reduced by β-reduction (`eval::apply_lambda`), keyed by the call's
    /// `(head-occurrence, argument-occurrences)`. β-reduction is a PURE function of the lambda body and
    /// the argument occurrences (both fixed node identities), so a given call site reduces to the same
    /// term every time — cache it. WITHOUT this memo a nested call chain `(f (f (f … 0)))` is
    /// EXPONENTIAL: each `apply_lambda` builds a FRESH copy of the body (new `StructId`s that no
    /// `type_of`/`core_of`/`resolved_of` memo can hit), and both `infer` AND `lower` reduce every call,
    /// each recursing into the reduced term whose nested calls re-reduce — O(2^depth) when the body
    /// duplicates its parameter, O(depth²) even for a single-use param. Memoized, each distinct call
    /// node reduces ONCE and every later demand returns the same reduced occurrence, so the downstream
    /// per-node memos hit and the whole chain is linear. `None` caches "not β-reducible here" (a
    /// non-lambda head / recursive / partial application — the caller falls back to its other path).
    /// Keyed by `(head, args.to_vec())` — the same identity `build_cache` uses for a constructor
    /// application, here for a function application.
    pub(crate) reduce_cache: crate::fxhash::FxHashMap<(StructId, Vec<StructId>), Option<StructId>>,

    /// Memo of the FAULTS a subtree collects (`infer::collect`) — the type-agreement rejections reachable
    /// from a node, keyed by its occurrence. A node's faults are a pure function of its resolved
    /// structure and the (memoized) types of its parts, so a node collected once need not be re-walked.
    /// This is the fault-collection sibling of [`reduce_cache`]: a nested call chain's fault walk
    /// descends each Apply's REDUCED body (`check_application` step 2), and that reduced body contains
    /// the inner call, whose collection re-descends its own reduced body — an EXPONENTIAL re-walk of the
    /// shared reduced terms even though the reduction itself is now cached. Memoized, each reduced node's
    /// faults are collected ONCE and every later demand copies the cached vec, so the fault walk over a
    /// call chain is linear. The stored faults already carry their stamped origins (`collect` stamps
    /// before returning), so replaying them is exact. Only a result collected WITHOUT hitting a
    /// depth/reduction limit is cached (a limit-clipped partial walk is not a node's true fault set).
    pub(crate) collect_cache: crate::fxhash::FxHashMap<StructId, Vec<crate::diag::Reject>>,

    /// The program-wide host-delegation SET: every effect-declaration occurrence some entrypoint (`export`)
    /// body delegates to the host via a `(host (E…) …)` — consulted once per RESIDUAL host-perform as a
    /// routing fallback (`effects::program_delegates_effect`). Computed by ONE walk of every export body
    /// collecting all delegated decls, then each query is an O(1) set membership test. Keying a cache by
    /// `decl` instead was still O(N²) for N DISTINCT delegated effects: each decl missed once → N full
    /// export-body walks (`body_has_host_delegating` ~86% self on an 800-effect delegation). `None` until
    /// the first query materializes it; the set is a pure function of the export bodies.
    pub(crate) delegated_effects: Option<crate::fxhash::FxHashSet<StructId>>,

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

    /// RIGID type variables for the duration of a single `compute_def_scheme` body solve — the def's OWN
    /// parameter type variables (the free vars of its `param_tys`). While set, `unify::freshen_free` LEAVES
    /// these vars unchanged instead of renaming them, so the param↔result element TIE survives the
    /// recursive-call arg-freshen in `apply_type` (a producer `List a -> Iter a` keeps its `a` tied rather
    /// than the result becoming a disjoint `∀a b`). A var NOT in this set — a genuinely-fresh local
    /// placeholder like `(None) : Option ?0` or `Map.empty : Map ?k ?v` — still freshens, so the
    /// occurs-check / branch-tie casualties are unaffected. This is the VAR-PROVENANCE distinction the
    /// shape-based discriminators could not make: a param/recursion var (rigid) vs a local placeholder
    /// (freshened). `None`/empty in every other context → `freshen_free` is byte-identical to before, so
    /// the hot application path pays only an `Option`-is-None check. Set+cleared around the body `type_of`
    /// in `compute_def_scheme`; never nested (a scheme solve does not recurse into another).
    pub(crate) scheme_rigid_vars: Option<crate::fxhash::FxHashSet<u32>>,

    /// The lambdas whose `expected_arrow_for_lambda` context-recovery is CURRENTLY ON THE STACK — the
    /// re-entry backstop for that ONE recovery. Path (3)/(3b) of `expected_arrow_for_lambda` reads the
    /// type of the storage context's head/param via `type_of`; for an UNANNOTATED parameter of a
    /// module-qualified or self-applied call, that `type_of` walks back to the SAME lambda's parameter,
    /// whose `type_of` re-enters `lambda_param_ty_from_context` → `expected_arrow_for_lambda` on the SAME
    /// lambda. Absent a guard this recurses until `descent_depth` hits `DESCENT_DEPTH_LIMIT` (~1024) — it
    /// TERMINATES on the 64 MB compiler thread but the ~1024 frames OVERFLOW the smaller browser/worker
    /// compile stack (a RangeError on a 3-line module program: `Temp.c-to-f(100)`). A lambda already
    /// mid-recovery recovers NOTHING (`None`) at re-entry — the same "no context hint, the param types
    /// `Any` and the body-solve grounds it" the `reduce_nodes` budget and the concrete-only domain checks
    /// already fall back to — so the cycle breaks at depth ~2 instead of at the limit. SCOPED to this
    /// recovery (NOT a global `type_of` guard) so it leaves the convergent recursive-parameter solve
    /// (`solving_params`/`solve_recursive_params`, which grounds a self-tail-recursive param by RE-reading
    /// its body) completely untouched. See `expected_arrow_for_lambda`.
    pub(crate) arrow_lambdas_in_progress: crate::fxhash::FxHashSet<StructId>,

    /// Guard against re-entering the recursive-parameter solve for a def already being solved — the
    /// solve types the body with a LOCAL env (not `type_of`), so a self-call reads the provisional
    /// signature rather than re-triggering; this set is the defensive backstop that a demand landing
    /// mid-solve returns without recomputing. Holds the def indices whose solve is on the stack.
    pub(crate) solving_params: crate::fxhash::FxHashSet<usize>,

    /// Guard against infinite recursion when re-typing a MUTUAL-RECURSION PARTNER's call result from its
    /// BODY (the SCC-freeze fix in `arg_ty_in_env`'s `SumPayload` arm): typing partner `dn`'s body demands
    /// `dac`'s body (the mutual edge), whose `child`-off-`dn` payload would re-demand `dn`'s body — this
    /// set holds the callee def indices whose body is being result-typed so the re-entry bottoms out at
    /// `Any` (the same `Any` the deferred scheme gives) instead of looping. Cleared as the stack unwinds.
    pub(crate) scc_result_typing: crate::fxhash::FxHashSet<usize>,

    /// The def indices whose SCHEME solve is currently on the stack — the re-entry backstop for
    /// `def_scheme` (a self-call demanding the scheme mid-compute reads `None`, typing as `Any`, the
    /// same behavior the base case absorbs). Distinct from a cached `None` in `def_schemes`: a genuine
    /// undetermined scheme and an in-progress one both look like `None` in the map, so re-entry can't be
    /// read from there. It ALSO gates caching — a `None` computed while a SIBLING scheme solve is still
    /// on this stack may be spurious (it read the sibling's provisional in-progress signature as `Any`,
    /// as a mutually-recursive pure dispatcher does), so it is NOT cached and a later demand recomputes
    /// it once the sibling is determined.
    pub(crate) solving_schemes: crate::fxhash::FxHashSet<usize>,
    pub(crate) seed_transitive: crate::fxhash::FxHashSet<usize>,

    /// The ROOT of the body currently being β-reduced / structurally COPIED (`eval::beta_reduce`),
    /// or `None` outside a reduction. Used by the capture-share exception: a pinned match-arm/pattern
    /// binder is SHARED (its `SumPayload` resolution preserved) only when its scrutinee is EXTERNAL to
    /// the body being copied — a genuine capture from an enclosing scope. If the scrutinee lies WITHIN
    /// this root, the scrutinee is itself being copied (a fresh occurrence), so the binder must be COPIED
    /// too, to re-resolve against the copied scrutinee — otherwise the shared binder reads the ORIGINAL,
    /// now-orphaned scrutinee (whose own param references have no slot in the copied function → the
    /// "parameter reference has no local slot" backend decline). Set/cleared around the outermost
    /// `beta_reduce` in `apply_lambda` / `copy_structural_pub`; a nested `beta_reduce` (a recursive copy
    /// of a sub-node) keeps the outer root, so `is_within` still tests membership in the whole copied body.
    pub(crate) reduction_root: Option<crate::ast::StructId>,

    /// CALL-SITE index for `infer::call_site_arg_types`: `callee def index → the argument-occurrence lists
    /// of every application (in ANY def body) whose head resolves to that callee`. Call-site inference
    /// (seed an open param from a caller's argument) needs "every call of `def`"; computing it by scanning
    /// EVERY def body per query was O(defs × program) → O(N²) for N mutually-recursive defs each needing
    /// the seed (a decoder pipeline: N pairs = 15.9s @1600). Built ONCE by one whole-program walk
    /// (`infer::build_call_site_index`), then each query is an O(1) map lookup. `None` until first
    /// materialized; a pure function of the resolved program (a def's body is fixed after load).
    pub(crate) call_sites_by_callee: Option<CallSiteIndex>,

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

    /// (A) STRICT heap-collection construction (#5194), CASE2: element-arg COMPUTATIONS decomposed out of a
    /// DEAD list/set/map ctor by `lower_let`, recorded here so the backend `Core::Seq` emit does NOT §283-
    /// elide them. A reached heap-collection ctor's args are STRICT — their traps must occur even when the
    /// collection is discarded — and (A) OVERRIDES the general §283 "an unobserved trap is elidable"
    /// affordance for exactly these args (v-spec-oracle). Semantically (A) requires only EVALUATING the
    /// trap-possible arg computation, NOT building the collection (an already-value/borrowed element raises
    /// no trap → contributes nothing → is left untouched, so there is no consume/reclaim and no double-free).
    /// So `lower_let` decomposes the ctor to its trap-possible scalar arg computations, marks them here, and
    /// sequences them (discarded) before the body; the Seq emit force-evaluates a marked stmt (its trap
    /// fires) and drops its scalar result. A memo of a lowering decision, like `kept_bindings`.
    pub(crate) strict_force_eval: crate::fxhash::FxHashSet<StructId>,

    /// LAMBDA-LIFTED closures — the body occurrences of `(fn …)` lambdas that survived lowering as a
    /// RUNTIME value (passed to a recursive callee, stored in a runtime cell) and were lifted to
    /// standalone wasm functions. In discovery order; a lambda's position here is its funcref-TABLE slot
    /// (the value a `Core::Closure` stores and a `Core::CallClosure` dispatches through). Populated by
    /// [`Db::lift_lambda`] during lowering (memoized — a lambda lifted once keeps its slot); read by
    /// `layout` to append the lifted functions to the emission order and build the table/element
    /// sections. A body used only at compile time (β-reduced away) never lifts, so this is empty for a
    /// program with no runtime closure — byte-identical to before. `DESIGN-runtime-closures-rcdzc.md` §3.
    pub(crate) lifted: Vec<crate::lower::LiftedLambda>,

    /// `body occurrence → its slot in [`lifted`]` — the O(1) dedup index for [`Db::lift_lambda`]. Lifting
    /// deduplicates by the lambda's `body` (one slot per distinct lambda), and the lookup was a LINEAR scan
    /// of `lifted` per lift → O(N²) for a program that lifts N distinct closures (a list/tuple of N escaping
    /// lambdas: ~N²/2 scan iterations — 18M at N=6400). This map makes each dedup O(1); kept in lockstep
    /// with `lifted` (an entry is inserted exactly when a new lambda is pushed).
    pub(crate) lifted_by_body: crate::fxhash::FxHashMap<StructId, usize>,

    /// `sum-decl occ → the set of sum decls its payloads TRANSITIVELY reach` — the memo behind the Rust
    /// backend's recursive-variant boxing cycle detector (`backend::rust::enums`). Deciding whether a
    /// variant is recursive asks "does this payload type reach the sum's own decl?", a graph reachability
    /// over the sum-reference edges. Computed per variant with a FRESH DFS, it was O(cycle) per call ×
    /// O(variants) per enum × O(enums) = O(N³) for N mutually-recursive sums (N=3200 cycle = 39s on the
    /// rust target, ~97% in `reaches_decl`). Memoizing the TARGET-INDEPENDENT forward-reachable set per
    /// decl (computed once, iteratively — no `visited`-clipped partial to mis-cache) makes each "does `s`
    /// reach `t`?" an O(1) set lookup. `None` until first materialized; a pure function of the type graph.
    pub(crate) sum_reachable:
        crate::fxhash::FxHashMap<StructId, std::rc::Rc<crate::fxhash::FxHashSet<StructId>>>,

    /// `sum-decl occ → the sum decls its payloads DIRECTLY mention` (the out-edges of the sum-reference
    /// graph) — the reusable half of [`sum_reachable`]. Each decl's out-edges need a `typeval_of` per
    /// payload (the profile's `project_meta`/`resolved_of` cost); computing them fresh inside every
    /// `sum_reachable_from(start)` re-did that per decl per start = O(N²) `typeval_of` for N sums in one
    /// cycle. Cached per decl (computed once → O(N) total), so the reachability closure is pure graph
    /// traversal over cached edges. `None` until first materialized; a pure function of the type graph.
    pub(crate) sum_out_edges: crate::fxhash::FxHashMap<StructId, std::rc::Rc<Vec<StructId>>>,

    /// The PROGRAM-WIDE typo-suggestion candidate pool per POSITION CLASS (0 = value, 1 = member-operand,
    /// 2 = type-expression) — the def / variant / effect / type / prelude names an unbound reference in
    /// that syntactic position could have meant (`resolve::nearest_name_suggestion` tiers 2-4). It is a
    /// pure function of the program, so building it FRESH per unbound occurrence (cloning every def name)
    /// made "N call sites of one missing name" (a forgotten import / renamed helper) O(N²) — N pool
    /// rebuilds × O(N) clones (a wide such file: ~45% malloc/realloc + the edit-distance over the pool).
    /// Cached here (built once per class), the per-occurrence work is just prepending the SMALL tier-1
    /// lexical bindings + the edit-distance scan (length-prefiltered). `None` until first materialized.
    pub(crate) suggest_pool: [Option<std::rc::Rc<Vec<String>>>; 3],

    /// Memo of the PROGRAM-WIDE typo-suggestion winner per `(unbound-name, position-class)` — the nearest
    /// name in `suggest_pool[class]` to that query, or `None` if none is within the typo cutoff. The
    /// edit-distance scan over the pool is O(pool) per query; N call sites of the SAME missing name (a
    /// forgotten import / renamed helper referenced N times) re-ran the identical scan → O(N²). Keyed by
    /// `(name, class)`, the pool winner is computed once per distinct query; the per-node lexical tier is
    /// combined against it afterward (a tiny scan). A pure function of the program + query.
    pub(crate) suggest_pool_winner: crate::fxhash::FxHashMap<(String, u8), Option<String>>,

    /// Memo of the nearest VARIANT name of a scrutinee sum to a mistyped match-pattern head — keyed by
    /// `(sum-decl occ, mistyped-key)`. `lower::enrich_pattern_head_suggestion` clones the sum's variant
    /// names + edit-distance-scans them per bad pattern; a WIDE sum (N variants) matched with a stale
    /// variant name from N sites (a renamed variant still named at many match arms) re-ran that O(variants)
    /// scan each → O(N²). Keyed by `(decl, key)`, the winner is computed once per distinct query. A pure
    /// function of the sum declaration + the mistyped key. The variant-suggest twin of `suggest_pool_winner`.
    pub(crate) variant_suggest_winner: crate::fxhash::FxHashMap<(StructId, String), Option<String>>,

    /// Memo of the TIER-2 "closest matches" LIST for a mistyped match-pattern head — the far-miss companion
    /// of `variant_suggest_winner`, keyed the same `(sum-decl occ, mistyped-key)`. When the mistyped head is
    /// too far for a confident single suggestion (a WRONG-SUM ctor — a valid variant of a DIFFERENT sum — is
    /// always a far miss), `enrich_pattern_head_suggestion` lists the scrutinee sum's closest variants via
    /// `suggest::closest_matches`, which SORTS all N variants by edit distance (O(N log N)). N wrong-sum
    /// match arms against a wide sum re-ran that per site → O(N² log N). Keyed by `(decl, key)`, the list is
    /// computed once per distinct query. A pure function of the sum declaration + the mistyped key.
    pub(crate) variant_closest_matches: crate::fxhash::FxHashMap<(StructId, String), Vec<String>>,

    /// Memo of the RECORD-FIELD "no field `k` — did you mean?" enrichment (`infer::no_field_reject`), keyed
    /// by `(reduced-record occ, mistyped-key)` → the `(confident-winner, hint-message)` pair. The enricher
    /// builds the record's O(fields) name list and edit-distance-scans it TWICE (`nearest` for the fix, then
    /// `did_you_mean` for the message), so a WIDE record with a renamed/typo'd field accessed from N sites
    /// re-ran that per access → O(N²). Keyed by `(record occ, key)`, computed once per distinct query — the
    /// record-field twin of `variant_suggest_winner`; a pure function of the record's field set and the
    /// mistyped key. Populated only when the operand reduces to a concrete record occurrence (the shared
    /// case that repeats); a type-only fallback re-scans (rare, not the hot repeat).
    pub(crate) no_field_suggestion:
        crate::fxhash::FxHashMap<(StructId, std::sync::Arc<str>), NoFieldSuggestion>,

    /// CAPTURED-reference occurrences: a body reference inside a lifted lambda that names a FREE VARIABLE
    /// (a binding from the lambda's creation scope), mapped to `(capture index, solved type)`. Recorded
    /// by `lower::lower_lambda_value` when the lambda is lifted; read when the LIFTED body is lowered so
    /// that reference produces a `Core::Captured` (an env-cell read) rather than following the ref to the
    /// out-of-scope binding. Keyed by the reference OCCURRENCE (unique per use). Empty for a program with
    /// no capturing closure. `DESIGN-runtime-closures-rcdzc.md` §3.
    pub(crate) captured_ref: crate::fxhash::FxHashMap<StructId, (usize, Ty)>,

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

    /// Quote-PATTERN nodes with a NON-FINAL `,@` splice — `` `(f ,@init ,last) `` — collected by
    /// [`crate::quote::reify_quotes`] (which detects them while desugaring a pattern-position quasiquote,
    /// then leaves the node un-reified). A `,@` binds the tail, so it is meaningful only as the FINAL
    /// element (`metaprogramming.md`); a non-final one is ill-formed. [`crate::compile::collect_faults`]
    /// reports each as CDZ0221 (the quote-pattern analogue of the binary-form CDZ0220). Empty for a
    /// program with no such defect.
    pub(crate) nonfinal_splice_patterns: Vec<StructId>,

    /// The resolved-form column. Filled only by [`crate::resolve`].
    pub(crate) resolved: Column<StructId, Resolved>,
    /// Test-only: count of clone-returning [`crate::resolve::resolved_of`] calls against THIS `Db` — the
    /// deterministic, noise-free metric for the borrow-family cleanups (a `resolved_of`→`resolved_ref`
    /// conversion at a dispatch/tag-test site removes one clone per call). A PER-`Db` counter, NOT a
    /// process-global `AtomicU64`: the count is a pure function of one program's compile, but a global
    /// shared across the parallel test harness is `fetch_add`-polluted by OTHER tests' concurrent
    /// `resolved_of` calls during the read window (they compile on their own worker threads into the same
    /// static) — so the reading inflated nondeterministically under load and the regression-guard test
    /// false-tripped. Scoped to one `Db`, every increment is single-owner (`resolved_of` holds `&mut Db`)
    /// and the test reads the field off the very `Db` it drove — zero cross-test contamination. See
    /// `a_wide_match_resolves_in_a_bounded_number_of_clones`.
    #[cfg(test)]
    pub(crate) resolved_of_calls: u64,
    /// Test-only: within-bucket `core_eq` comparisons the wasm CSE class-partition
    /// (`select::collect_cse_candidate_groups`) made against THIS `Db` — the noise-free signal that the
    /// partition buckets candidates by a cheap shallow `core_hash_key` and compares only WITHIN a bucket,
    /// so a WIDE all-distinct arithmetic body makes ~O(#candidates) compares (each its own bucket → ~0),
    /// NOT the ~cands²/2 the old all-pairs scan made. A PER-`Db` counter for the SAME reason as
    /// [`Self::resolved_of_calls`]: a process-global `AtomicU64` is `fetch_add`-polluted by the parallel
    /// test harness's OTHER concurrent compiles during the read window (the emit path runs on a scoped
    /// worker thread, incrementing a shared static), so the reading inflated nondeterministically under
    /// load. Scoped to one `Db` (`collect_cse_candidate_groups` holds `&mut Db` → single-owner increments)
    /// and surfaced out of the emit path via the `#[cfg(test)]` `CompileOutput::cse_partition_core_eq_calls`
    /// field, so the test reads a value from exactly one compile. See
    /// `a_wide_arithmetic_body_partitions_cse_candidates_in_bounded_time`.
    #[cfg(test)]
    pub(crate) cse_partition_core_eq_calls: u64,
    /// Test-only compile-cost counter: how many times [`crate::lower::value_range_uncached`] ran (a
    /// `value_range` query that missed its refinement-free memo). Surfaced via
    /// `CompileOutput::value_range_uncached_calls` for the regression guard
    /// (`value_range_stays_linear_on_a_runtime_binding_chain`) to assert LINEAR (not quadratic) growth.
    #[cfg(test)]
    pub(crate) value_range_uncached_calls: u64,
    /// Test-only compile-cost counter: how many times [`crate::effects::param_apply_extra_handled`] ran its
    /// body (a call that missed its `(callee_body, arity, depth)` memo). Surfaced via
    /// `CompileOutput::param_apply_extra_handled_calls` for the regression guard
    /// (`param_apply_extra_handled_stays_linear_on_a_nested_applied_lambda_chain`) to assert LINEAR (not
    /// exponential) growth — pins the seq-203 #5755 memo.
    #[cfg(test)]
    pub(crate) param_apply_extra_handled_calls: u64,
    /// Test-only compile-cost counter: how many times [`crate::backend::wasm::select::is_cse_shareable`] ran
    /// its inner body (a query that MISSED the `Self::is_cse_shareable_memo`). Surfaced via
    /// `CompileOutput::is_cse_shareable_uncached_calls` for the regression guard
    /// (`is_cse_shareable_stays_linear_on_a_nested_expression`) to assert LINEAR (not quadratic) growth — the
    /// straight-line CSE driver queries the predicate per candidate node, so without the id-keyed memo a
    /// deeply-nested expression re-walks overlapping subtrees per enclosing node (O(N²)+). Scoped to one `Db`
    /// (the emit path holds `&mut Db`) so the parallel test harness cannot pollute it.
    #[cfg(test)]
    pub(crate) is_cse_shareable_uncached_calls: u64,
    /// The solved-type column. Filled only by [`crate::infer`].
    pub(crate) types: Column<StructId, Ty>,
    /// The ground TYPE-VALUE memo — the read-through cache for [`crate::eval::typeval_of`]. Distinct from
    /// [`Self::types`] (which memoizes `type_of`, the type OF a node's value): this holds the type-VALUE a
    /// node DENOTES (`(: b (Box t))`'s `(Box t)` → `Ty::Sum{Box, [Var t]}`). `typeval_of` is env-free and
    /// deterministic per `StructId` once the load-time newtype precompute+fixpoint completes, so its result
    /// caches per node. Without it a wide structural-record type ANNOTATION (`{tree, resolved: Map(Int64,
    /// Resolved), …}`) re-reduces from scratch at every one of its ~1M referencing uses through the
    /// resolve→infer→lower chain, exhausting the per-query reduction budget → a spurious CDZ0999 "does not
    /// reduce within limits" on a whole-project check (the DB-records self-host case). TRAP: STALENESS: filled
    /// ONLY when [`Self::typeval_memo_live`] is set — the newtype precompute reduces payloads BEFORE
    /// `newtype_inner` is complete (a pre-normalization `Ty::Sum`), so the memo must not capture those; the
    /// flag flips true right after the post-population evict at load's end. Invalidated per node by
    /// [`crate::resolve::forget_subtree`] alongside `resolved` (a lowering-time re-parent changes the node's
    /// resolved form, so its denoted type-value can change too).
    pub(crate) typeval: Column<StructId, Ty>,
    /// Whether [`Self::typeval`] may be filled (see its doc). False during the load-time newtype precompute
    /// — which reduces payloads against an incomplete `newtype_inner` — and true for every real query after.
    pub(crate) typeval_memo_live: bool,
    /// The core-form column. Filled only by [`crate::lower`].
    pub(crate) core: Column<StructId, Core>,
    /// Pass-installed CORE OVERRIDES (the `PassManager`/`CorePass` seam, `crate::opt`). A backend-independent
    /// Core-IR optimization pass rewrites a node's core form by installing an override here; `lower::core_of`
    /// consults this map FIRST (before the memoized column), so the override wins and both backends inherit
    /// the rewrite (`DESIGN-tiered-optimization-levels-rcdzc.md` §9a). Keyed by the SAME `StructId` the core
    /// column uses, so a pass keys by the node's stable id. EMPTY in the default (O0/unoptimized) pipeline —
    /// then `core_of` behaves exactly as before (byte-identical emit), which is the level-equivalence
    /// baseline. A pass MUST keep every override behavior-preserving (the `CorePass` correctness bar); the
    /// override is consulted by the same producers/walks that read `core_of`, so an installed rewrite is seen
    /// uniformly. Cleared whenever the core column would be (incremental re-lower), since an override is a
    /// derived fact over the same input.
    pub(crate) core_override: crate::fxhash::FxHashMap<StructId, Core>,
    /// Memo for LOCAL LAMBDA-LIFT (a recursive do-local function that CAPTURES an enclosing parameter):
    /// callee def index → the ordered capture REFERENCE occurrences appended as extra trailing call
    /// arguments (whose signature entries were appended to `defs[callee].params`, so `select_function`
    /// slots the captured param in the callee's own frame and the body's otherwise slot-less `Core::Param`
    /// resolves without a rewrite). Computed ONCE the first time a recursive call to the callee is lowered,
    /// then memoized: recomputing AFTER the params were extended would classify the captures as OWN params
    /// and drop them, corrupting the self-call's arity. EMPTY for a non-internal callee, one with no
    /// captures, or an unsupported (non-parameter) capture → no extra args → byte-identical to before, and
    /// the honest CDZ0900 recursive-local-capture decline still fires for the unsupported shapes. Tied to
    /// the `defs[].params` mutation (both persist together, neither reset on a core-column re-lower).
    pub(crate) local_lift_captures: crate::fxhash::FxHashMap<usize, Vec<StructId>>,
    /// Memo of RECURSIVE-EFFECTFUL SPECIALIZATIONS (`crate::effects` E3): a recursive function called
    /// under a handler context is emitted ONCE per `(def-body-occ, handler-context-key)` as a synthesized
    /// `f#ctx` def — its state threaded as trailing parameters (`DESIGN-effects-rcdzc.md` §4.3). The key
    /// is the recursive callee's body occurrence plus a RESOLVED handler-context identity string (the
    /// discharged ops + their arm occurrences — NOT `format!("{:?}", body)`); the value is the `db.defs`
    /// index of the synthesized specialization, so the same call under the same context reuses one def.
    pub(crate) effect_specializations: crate::fxhash::FxHashMap<(StructId, String), usize>,

    /// The synthesized specialization NAMES that use the MULTI-VALUE-RETURN calling convention (repro-1):
    /// `f#ctx` returns `(value, out-state-per-slot)` instead of a bare value, so a later sibling self-call
    /// can thread the recursion's advanced out-state. EVERY caller of such a spec — the recursive self-call
    /// arm AND the top-level handle reduction — must project `.0` for the value (and, in the self-call arm,
    /// `.{slot+1}` for each threaded state). A spec NOT in this set uses the ordinary single-return
    /// convention (id-sum/countdown), unchanged. Populated by `specialize_recursive`.
    pub(crate) multivalue_specs: std::collections::HashSet<String>,

    /// Per synthesized specialization NAME, the CAPTURED enclosing-fn param names that its handler arms
    /// reference free (e.g. a handler arm `converse(q,s) => resume(tool,0)` where `tool` is the enclosing
    /// `run-with`'s param — NOT the arm's own params/state, NOT the recursive def's params). Such a name is
    /// threaded as an EXTRA specialized param (after the original params, before the trailing state params),
    /// and every self-call — plus the initial call from the handle body — passes it through UNCHANGED (it is
    /// constant across the recursion). Without this, the free name resolves against the synthesized `f#ctx`
    /// sig (which lacks it) → a spurious CDZ0101 (v-agent-harness merged-nested multi-arm dogfood).
    /// Populated by `specialize_recursive`; read by the self-call rewrite arm.
    pub(crate) effect_spec_captures: std::collections::HashMap<String, Vec<String>>,

    /// Recursive-effectful callees whose FINAL out-state is OBSERVED by a later item in the CALLER's strict
    /// spine — a `(do (run-ops …) (Prim.run 0))` where the trailing perform reads the state the recursion
    /// advanced. Keyed like `effect_specializations` (`(callee-body-occ, handler-context-key)`). Such a
    /// callee MUST specialize in MULTI-VALUE mode so its advanced out-state threads to the observing
    /// continuation; the single-return convention drops it (returns the incoming state unchanged), silently
    /// miscompiling the observer to the PRE-recursion state (task #15, the cross-fn-fold out-state drop).
    /// The mode decision (which reads the callee's OWN body) cannot see a CALLER-side observation, so
    /// `reduce_handle` scans the handle body up front and records the marks here for `specialize_recursive`
    /// to consult. Only UPGRADES a threadable callee to multi-value — a non-threadable one is left as-is.
    pub(crate) force_multivalue: std::collections::HashSet<(StructId, String)>,

    /// Node ids that lie within a HANDLER-ARM-TAIL / `#st`-threaded region that `reduce_handle` produced
    /// (populated by [`crate::effects::mark_handler_region`] on the reduced form). Consulted by CASE2's
    /// strict-heap-ctor decompose in `lower_let` ([`crate::effects::node_in_handler_region`]): a dead
    /// list/set/map ctor whose enclosing `let` sits in such a region must NOT be decomposed + `Core::Seq`-
    /// wrapped, because the handler tail/`#st`-drop/per-dispatch-reclaim lowering recognizes sequencing via
    /// `do` FORMS and NOT `Core::Seq`, so the wrapper perturbs it (olc1/cst1/sga1 per-dispatch leaks). The
    /// skip is conformance-neutral: a pure-trap-possible dead ctor inside a handler arm is rare (an effectful
    /// arg is already force-kept), so the strict-eval it would add is a rare known-gap, not a broad loss.
    pub(crate) handler_region_nodes: std::collections::HashSet<StructId>,

    /// Members of a MUTUALLY-RECURSIVE SCC being group-specialized in MULTI-VALUE mode together (the
    /// group-aware recursive-perform fold). Keyed `(member-body-occ, handler-context-key)` like
    /// `force_multivalue`. When `specialize_recursive` finds the entry callee's SCC has a mutual partner
    /// whose out-state a later spine item observes, it records EVERY SCC member here up front, so each
    /// member: (a) computes `multivalue = true` (threads its body via `thread_returning_tuple`), and (b)
    /// skips the `mutual_partner_precedes_observation` clean-decline (that guard is the SINGLE-return
    /// floor — with the whole group multi-value, a mutual-partner call is let-bound + out-state-projected
    /// like a self-call by the head-agnostic recursive-call arm). Cleared when the group finishes (or on
    /// any member's decline, so a partial group never leaks). Distinct from `multivalue_specs` (which is
    /// keyed by the SYNTHESIZED spec NAME and read by the call-rewrite arm); this is keyed by the ORIGINAL
    /// body so the per-member mode decision inside `specialize_recursive` can consult it.
    pub(crate) group_multivalue_bodies: std::collections::HashSet<(StructId, String)>,

    /// Memo of `effects::mutual_scc_of` — the mutually-recursive SCC (def indices) containing a callee,
    /// keyed `(callee_def, handler-context-key)`. SCC membership is program-STATIC (the call graph does not
    /// change during a fold), and the discharged-op filter is fixed by the context key — so the result is
    /// stable and computed ONCE per (callee, ctx). Removes the double-compute on the group-entry path (the
    /// predicate + the member-registration call the same `mutual_scc_of`) and any re-derives across sibling
    /// SCC members. `mutual_scc_of` is a forward BFS + a per-def reaches-BFS over the call graph, the first
    /// call-graph-scaled cost in the effect fold (v-compiler-perf advisory on #2877); memoizing keeps it off
    /// the hot path. Populated + read only inside `mutual_scc_of`.
    pub(crate) mutual_scc: crate::fxhash::FxHashMap<(usize, String), Vec<usize>>,

    /// Memo of `effects::subtree_performs` — whether the subtree at a node reaches a discharged perform (a
    /// `resume`, or a call into a discharged effect) under a given handler context. Keyed by `(node,
    /// handler-context-key)` (the same resolved-identity string `effect_specializations` uses). The
    /// frame-free effect fold's `strongly_pure`/`pure_hole` classifiers call `subtree_performs` at MANY
    /// nodes as they descend a handle body — `strongly_pure` re-ran the WHOLE-subtree walk at every node it
    /// visited, so a deep body (an N-perform nested-`let` chain) was O(N²) in the scan and the fold O(N³).
    /// The memo makes each node's verdict compute once, so repeat queries are O(1). Keyed on the context
    /// so a node reused under a different discharged-op set is not conflated.
    pub(crate) subtree_performs_cache: crate::fxhash::FxHashMap<(StructId, String), bool>,

    /// Nodes already fully walked by [`Db::collect_reduced_callables`]. `register_reduced_callables` runs
    /// after EVERY `apply_lambda` β-reduction to discover do-local recursive defs in the reduced term; the
    /// walk descends the WHOLE reduced subtree. A deeply-nested reduction (each `apply_lambda` returns a
    /// progressively-larger term — e.g. a `((. d op0) ((. d op1) … acc))` dictionary-projection chain) had
    /// each of N reductions re-walk an O(N)-deep term = O(N²). A node's structure is immutable once built,
    /// so once walked it yields no new candidates on a re-walk — skip an already-visited node. (A node id is
    /// never reused with a different meaning; a synthesized node gets a fresh id, so the memo cannot
    /// mislead. This is the fix-30 `reached_visited` pattern applied to the reduced-callable scan.)
    pub(crate) reduced_callable_walked: crate::fxhash::FxHashSet<StructId>,

    /// Memo of TYPE MONOMORPHIZATIONS (`crate::lower`, recursive-generic monomorphization): a recursive
    /// GENERIC function called at concrete argument types is emitted ONCE per `(def-body-occ,
    /// instantiation-key)` as a synthesized copy whose parameters are re-annotated with the concrete
    /// types (`DESIGN-recursive-generic-monomorphization-rcdzc.md`). The key is the recursive callee's
    /// body occurrence plus a RENDERED concrete-signature identity (the instantiated param types joined
    /// — NOT `format!("{:?}", body)`); the value is the `db.defs` index of the synthesized copy, so the
    /// same def called at the same types reuses one function. A non-recursive call inlines (never reaches
    /// here); a monomorphic recursive def has no free scheme var (never specializes). Empty for a program
    /// with no recursive-generic call — byte-identical to before.
    pub(crate) type_specializations: crate::fxhash::FxHashMap<(StructId, String), usize>,

    /// The `const`-parameter fingerprint recorded for each specialization, keyed by `(orig_body, param
    /// NAME)`. Set when `type_specialize` synthesizes a spec for a def with a const param; read when a
    /// SELF-RECURSIVE const re-pass of THAT def's const param is later lowered (demand-driven, possibly
    /// long after the enclosing spec was built). A re-pass on a MIXED-match recursive arm has an UNBOUND
    /// `step` in the orphan copy, so probing `core_of` yields a Poison → no `|clos` suffix → a DIFFERENT
    /// memo key → a divergent second spec whose body carries the unbound `step` (the const-param-drop /
    /// CDZ0101 bug v-iterators' filter-map hit). Inheriting the recorded fingerprint makes the recursive
    /// call's key MATCH the enclosing spec's, so the recursion closes on ONE spec (re-enters the working
    /// instance) and the orphaned body never lowers. Empty for a program with no recursive-const spec —
    /// byte-identical to before. Keyed by `(orig_body, name)` (not a stack) because the recursive call's
    /// lowering is demand-driven and not lexically nested inside the spec's construction.
    pub(crate) const_repass_fp: crate::fxhash::FxHashMap<(StructId, String), String>,

    /// The fingerprint of the MOST RECENT `const`-CLOSURE specialization synthesized for each recursive
    /// def, keyed by `orig_body`. A TERMINATION BACKSTOP for the derived-closure divergence: a
    /// self-recursive def that re-passes a closure DERIVED FROM its own const param (`(def (bad (const
    /// step) …) (bad (fn (x) (+ (step x) 1)) …))`) builds, at each recursion depth, a closure that NESTS
    /// the previous depth's closure — so its fingerprint STRICTLY EXTENDS the immediately-preceding spec's
    /// (contains it as a substring AND is longer): 22→46→70→… unbounded → a non-terminating spec unroll
    /// that HANGS the compile. A GENUINE const-closure consumer — even a whole `map().filter().fold()`
    /// fusion pipeline — passes DISTINCT, unrelated closures to a driver (probed: `drive`'s successive fps
    /// `nstep;|clos1`, `(nfn;…)`, an annotated form — NONE extends its predecessor across the entire
    /// iterators suite). So declining a re-pass whose fp strictly EXTENDS the last spec's, keyed per
    /// `orig_body`, catches ONLY the divergence — at the const-arg gate, before the divergent (malformed)
    /// spec is built, so the reject is the coded CDZ0201. Per-compile-scoped (a fresh `Db` per `compile`
    /// starts this empty), so it never conflates specs across files/@tests. Keys on the IMMEDIATELY-
    /// preceding spec of the SAME def (a single fp per orig_body), NOT any-pair containment (which
    /// false-fires on independent adapter specs where a later `nstep;|clos1` contains an earlier `nstep;`).
    pub(crate) last_const_closure_fp: crate::fxhash::FxHashMap<StructId, String>,

    /// A HUMAN-READABLE record of each concrete instantiation `type_specialize` synthesized, in
    /// synthesis order — one [`Instantiation`] per DISTINCT specialization (appended at the memo miss,
    /// beside the `type_specializations` insert). The memo answers "have I already built this instance?";
    /// this list answers "what are ALL the instances of definition X, and at what concrete arguments?" —
    /// the data the `Instantiations` query reports. Its per-argument descriptions render a `const`
    /// dictionary argument as readable source (not the memo's opaque fingerprint), so a report can show
    /// which concrete dictionary an ad-hoc-polymorphic call baked in. Empty for a program with no
    /// recursive-generic / type-valued / `const` instantiation — no cost to a monomorphic program.
    pub(crate) instantiations: Vec<Instantiation>,

    /// The `db.defs` indices of definitions the lowerer INLINED (β-reduced at a call site) at least once
    /// — the companion of `instantiations` for the OTHER monomorphization path. A non-recursive call to a
    /// named def folds the callee body into the call site (`eval::apply_lambda`, `lower`'s β-reduce arm)
    /// with NO `Core::Call` and no emitted function; a nullary def called by name inlines the same way via
    /// the zero-argument identity path. Recorded at those inline sites (keyed on `callee_def_index` / the
    /// nullary def-by-body), so the `Instantiations` query can report a def's DISPOSITION — inlined vs
    /// specialized vs emitted-as-a-standalone-call vs unreferenced. Populated during lowering (an inline
    /// leaves no other trace), forced complete by `layout::force_monomorphize`. Empty until a call is
    /// lowered — no cost to a program that is never lowered/queried.
    pub(crate) inlined: crate::fxhash::FxHashSet<usize>,

    /// The `db.defs` indices of definitions that were EMITTED AS A STANDALONE FUNCTION and CALLED — a
    /// `Core::Call { callee }` names them (a recursive def, an `inline-never` def, or a specialization).
    /// The complement of `inlined`: a def reached through a real runtime call rather than folded away.
    /// Recorded by `layout`'s reachable-callee walk (`collect_call_callees`) — the same walk
    /// `force_monomorphize` already runs — so the `Instantiations` query can report "emitted" as a
    /// disposition. Populated only when a body is lowered for reachability/monomorphization; empty
    /// otherwise (no cost to a plain compile that does not run the query).
    pub(crate) called: crate::fxhash::FxHashSet<usize>,

    /// Source-def → synthesized-copy links for defs the ACCUMULATOR transform rewrote (`accum::introduce`,
    /// at load): a linear non-tail recursion `f` is rewritten to seed a fresh tail-recursive copy `f$acc`
    /// (which the loop transform then compiles to a constant-stack loop), so the SOURCE `f`'s own body
    /// folds to a seed call while the emitted recursion lives under the copy's name. This map lets the
    /// `Instantiations` query report `f`'s disposition as `transformed→f$acc` — truer than the literal
    /// `inlined` (the seed wrapper folds) for "is there a function realizing f's recursion?". Keyed by the
    /// SOURCE def index → the copy def index. Empty for a program with no accumulable recursion.
    pub(crate) transformed: crate::fxhash::FxHashMap<usize, usize>,

    /// The NAME OCCURRENCES of parameters declared `const` — an EXPLICIT compile-time parameter (`(def (f
    /// (const (: d T)) …) …)`, `DESIGN-…-monomorphization-rcdzc.md` Addendum 3). A `const` param MUST be
    /// compile-time-known at each call: it is folded + inlined into a specialized copy and ERASED from the
    /// emitted runtime signature (a non-foldable argument is a coded compile error). Populated ONCE by
    /// `strip_const_params` at load (which also unwraps the `(const …)` binder in place, so every
    /// downstream reader sees a plain binder), keyed by the inner binder's `param_name_occ` — the identity
    /// `type_specialize` / the gate test against a def's params. Empty for a program with no `const` param
    /// (byte-identical to before). This replaces the implicit dict-sniffing heuristic: the AUTHOR declares
    /// what is compile-time, the compiler obeys.
    pub(crate) const_params: crate::fxhash::FxHashSet<StructId>,

    /// The RAW-CONSTRUCTION node ids inside the synthesized `__invariant_construct_<T>` defs — each
    /// `((. T V) __inv_p)` a checked constructor builds. These are the construct sites `lower_sum_new` must
    /// EXEMPT from the `@invariant` establish divert: they ARE the checked constructor's own construction, so
    /// routing them back through `__invariant_construct_<T>` would recurse forever. Populated ONCE by
    /// `invariant_establish::synthesize` at load (append-only synthesis keeps the ids stable through the later
    /// load passes). Empty for a program with no `@invariant` newtype. See [`crate::invariant_establish`].
    pub(crate) invariant_exempt_ctors: crate::fxhash::FxHashSet<StructId>,

    /// The BODY occurrences of definitions marked `inline-never` — an INLINE-POLICY marker
    /// (`DESIGN-…-monomorphization-rcdzc.md` Addendum 4). The default is always-inline (every
    /// non-recursive call β-reduces); `inline-never` forces the def to be EMITTED as ONE real wasm
    /// function, every call a `Core::Call` (routed through `emit_call_or_specialize`, so a generic /
    /// `const`-param `inline-never` def still SPECIALIZES — "avoid the inline but keep polymorphism").
    /// Populated ONCE by `strip_inline_policy` at load (which unwraps the `(@ inline-never (def …))`
    /// annotation in place, so every downstream reader sees a plain `(def …)`), keyed by the def's BODY occ — the
    /// identity `lower`/`layout` (`def_index_by_body`) already use. Empty for a program with none.
    pub(crate) inline_never: crate::fxhash::FxHashSet<StructId>,

    /// EMIT-ONCE per-callee decision MEMO (operator-directed "emit_shared" column): caches the PER-CALLEE part
    /// of `lower::should_emit_once_by_cost` — `true` iff the callee is a large-enough, monomorphic, non-`const`,
    /// high-fan-out def (body-size ≥ INLINE_COST_THRESHOLD · scheme monomorphic · no const-param · call-site-count
    /// ≥ INLINE_MIN_CALLERS). Keyed by `callee` def index. The heuristic previously RECOMPUTED all of this on the
    /// hot lower path at EVERY call site (a `bounded_node_count` walk + scheme resolve + call-site-count each
    /// time); this memoizes it once per callee. The PER-ARGS runtime-binding SOUNDNESS gate stays per-call-site
    /// (it depends on the actual args, not the callee, so it is NOT memoized here). A general query-DB memo column
    /// (the operator's "column not ad-hoc cache" steer), consistent with the other memo columns.
    pub(crate) emit_shared: crate::fxhash::FxHashMap<usize, bool>,

    /// The BODY occurrences of definitions marked `inline-always`. Today a NO-OP (the default already
    /// always-inlines), recorded so the forthcoming cost heuristic (deferred) can treat it as the override
    /// "ignore the cost model, always fold". Populated by `strip_inline_policy` alongside `inline_never`.
    /// An `inline-always` on a RECURSIVE def is a conflict (it cannot inline) — a coded reject.
    pub(crate) inline_always: crate::fxhash::FxHashSet<StructId>,

    /// The BODY occurrences of definitions marked `@test` — the UNIT-TEST hoist markers a `cdz test`
    /// build reads (`strip_annotations`). A test build exports every `@test` NULLARY def as a boundary
    /// entry (in place of the program's `(export …)` clauses); an ordinary build ignores this set (a
    /// `@test` def is just an unexported def — dead, dropped by reachability — so the report effect it
    /// performs never reaches the normal component's import set). Keyed by BODY occ like the inline sets.
    pub(crate) tests: crate::fxhash::FxHashSet<StructId>,

    /// The `@tag("…")` string tags a definition carries, keyed by its BODY occurrence (like `tests`).
    /// A `@tag("slow")` annotation records `"slow"` here; a def may carry several (`@tag("slow")
    /// @tag("net")` → `["slow", "net"]`), accumulated in annotation order. A tag is INDEPENDENT metadata
    /// — it does NOT imply `@test` — but its load-bearing use is filtering the `@test` set: `cdz test
    /// --tag slow` runs only the tests whose def carries the `"slow"` tag (`Db::tags_of`). Empty for a
    /// def with no `@tag`. The tag surface is `@tag("string")` (a call-style annotation whose name
    /// position is the application `(tag "string")`); `strip_annotations` reads that shape.
    pub(crate) tags: crate::fxhash::FxHashMap<StructId, Vec<String>>,

    /// The `@exhaustive` set — property tests the `cdz test` runner drives over their ENTIRE finite input
    /// domain (every combination of the scalar parameters), not by random sampling; keyed by BODY occ like
    /// `tests`. A pass over the full domain is a PROOF for that property, the total-coverage counterpart to
    /// sampled property testing (`property-based-testing.md` §Exhaustive). Read by `Db::is_exhaustive`. An
    /// `@exhaustive` def is ALSO in `tests` (it is a test), so `test_defs` hoists it with no extra `@test`.
    pub(crate) exhaustive: crate::fxhash::FxHashSet<StructId>,

    /// `(@ (tag …) …)` heads whose arg is not exactly one STRING — malformed `@tag` annotations. Read by
    /// `Db::malformed_tag_forms` to REJECT them in `collect_faults` (a silently-dropped tag masks the error).
    pub(crate) malformed_tags: Vec<StructId>,

    /// `@requires(pred)` PRECONDITION predicate occurrences per annotated def (BODY occ → the predicate
    /// `StructId`s, in annotation order). The verification layer (Inc-b) denotes these into HOL obligations;
    /// the proof-guided-elision oracle looks up a checked-arith node's precondition here. Recorded by
    /// `strip_annotations` (b4a); empty in a build with no `@requires`. Read by `Db::requires_of`.
    pub(crate) requires: crate::fxhash::FxHashMap<StructId, Vec<StructId>>,

    /// `@ensures(pred)` POSTCONDITION predicate occurrences per annotated def (BODY occ → predicate
    /// `StructId`s). The predicate is over the def's params + the implicit result binder `it`. Denoted +
    /// discharged by the verification layer (Inc-b); recorded by `strip_annotations` (b4a). Read by
    /// `Db::ensures_of`.
    pub(crate) ensures: crate::fxhash::FxHashMap<StructId, Vec<StructId>>,

    /// `@invariant(pred)` DATA-TYPE-INVARIANT predicate occurrences per annotated type declaration (the
    /// `(type …)` decl occ → the predicate `StructId`, over the value binder `it`). The DATA-level member of
    /// the verification-annotation family (design §10): a property EVERY value of the type maintains, verified
    /// (establish/preserve) + optimized-against + used as the generation constraint for `@test`. Unlike
    /// `@requires`/`@ensures` (which annotate a `(def …)` and key by BODY occ), `@invariant` annotates a
    /// `(type …)` and keys by the TYPE decl occ. Recorded by `strip_annotations`; empty in a build with no
    /// `@invariant`. Read by `Db::invariant_of` (v-property-testing's `gen<T>` seam consumes it). A type may
    /// declare at most one `@invariant` here (a later stacked one overwrites; a conjunction is the surface).
    pub(crate) invariants: crate::fxhash::FxHashMap<StructId, StructId>,

    /// `@requires`/`@ensures`/`@invariant` heads with not-exactly-one predicate argument (malformed by
    /// ARITY). Read by `Db::malformed_verify_forms` to REJECT them in `collect_faults` (CDZ0201), like
    /// `malformed_tags`.
    pub(crate) malformed_verify: Vec<StructId>,

    /// TRANSIENT flow-sensitive value-range REFINEMENTS, active only during wasm emit. A stack of
    /// frames, each mapping a variable's binder occurrence (a `Core::Param`/`LocalRef` `binder`) to a
    /// range `[lo, hi]` KNOWN to hold in the current control-flow branch (`hi = None` = unbounded above).
    /// Pushed when emit enters the `then`/`else` of an `if` whose condition compares a variable against a
    /// constant (`(< n 2)` refines `n` to `[MIN, 1]` in the then-branch, `[2, MAX]` in the else), popped
    /// on exit. `value_range` consults the TOP frame so a guard-elision check (`arith_provably_in_range`,
    /// etc.) sees the branch-narrowed range — eliding the dead underflow guard on `(- n 1)` inside `(> n
    /// 0)`. EMIT-ONLY and NOT memoized: `value_range`/the guard fns are recomputed per call (never cached),
    /// so a transient refinement can never poison a query result; the const-fold callers in `lower` run
    /// with this EMPTY (no branch context), so their behavior is unchanged. A frame is the FULL set of
    /// refinements in scope (each `if` merges its parent's frame with its own), so `value_range` reads
    /// only `last()`.
    ///
    /// The frame value is a [`ValueFact`] (a product-of-facets over-approximation), of which slice 1
    /// populates only the `int_range` facet — a behavior-identical widening of the former bare
    /// `(i64, Option<i64>)` tuple. `refined_range` still projects out `int_range` so the interval
    /// consumers (`value_range`, the guard-elision fns) are unchanged.
    pub(crate) range_refinements: Vec<crate::fxhash::FxHashMap<StructId, ValueFact>>,
    /// Memo for [`crate::lower::value_range`] — used ONLY when `range_refinements` is empty (no active
    /// flow-sensitive refinement), where `value_range` is a PURE function of the node's core. `value_range`
    /// recurses `LocalRef → initializer`, so over a sequential-dependency chain it re-walked all predecessors
    /// per node (O(N²) compile-time — a `--warm-only` probe showed near-quadratic growth on N runtime lets).
    /// Caching the refinement-free result makes it O(N). Refinement-active calls bypass this entirely.
    pub(crate) value_range_memo: crate::fxhash::FxHashMap<StructId, Option<(i64, Option<i64>)>>,
}

impl Db {
    /// Build a database over a decoded program: run the one cheap top-level scan (defs + exports),
    /// leave every derived column empty. Nothing below the top level is touched until a query demands
    /// it.
    pub fn load(ast: Arenas) -> Db {
        Db::load_linked(ast, None)
    }

    /// Install a pass-computed CORE OVERRIDE for `id` (the `crate::opt` `CorePass` seam). After this,
    /// `lower::core_of(db, id)` returns `core` instead of the lowered/memoized form, so a backend-independent
    /// optimization pass rewrites the Core-IR once and BOTH backends inherit it
    /// (`DESIGN-tiered-optimization-levels-rcdzc.md` §9a). The caller (a `CorePass`) MUST keep the override
    /// behavior-preserving — this is pure mechanism, no correctness check. Overriding a node whose column
    /// slot is already filled is fine: the override is consulted first, so it wins on the next read.
    pub fn install_core_override(&mut self, id: StructId, core: Core) {
        self.core_override.insert(id, core);
    }

    /// Whether any core override is installed (a pass has run and rewritten at least one node). `false` for
    /// the default/unoptimized pipeline — then `core_of` is byte-identical to before the seam existed.
    pub fn has_core_overrides(&self) -> bool {
        !self.core_override.is_empty()
    }

    /// Build a database over a decoded program that MAY be a linked multi-file package. `linkage` is
    /// `Some` only for a package of >1 file (`DESIGN-package-linking.md`): it carries the `StructId →
    /// file` demux table and each file's import/export surface, from which this derives the file-scoped
    /// resolution table ([`FileScopeTable`]). `None` is the ordinary single-file compile — flat
    /// namespace, byte-identical to before linking existed. Everything else about load is unchanged; a
    /// linked package is just one bigger arena with a resolution overlay.
    pub fn load_linked(ast: Arenas, linkage: Option<crate::link::Linkage>) -> Db {
        let mut ast = ast;
        // Peel every `(comment "…" <form>)` wrapper to its inner `<form>` FIRST — before any other strip
        // reads the form. A leading `//` comment on a form reifies (by the reader) to `(comment "text"
        // <form>)`; the comment is SEMANTICALLY INERT (self-hosting-surface.md — the compiler sees through
        // comments as it sees through docs), so an un-peeled wrapper would make `comment` an unknown
        // top-level declaration head (the wrapped `def`/`type`/`export` invisible → "unbound name comment"
        // + the def's name unbound). Done in place (the wrapper node adopts the inner form's children, ids
        // unchanged) and FIRST, so `strip_def_docs`/`strip_const_params`/`strip_annotations` + the top-level
        // scan all see the plain form. Mirrors `strip_annotations`'s in-place unwrap.
        strip_comments(&mut ast);
        // Normalize away a leading `(doc …)` on every `(def …)` BEFORE anything reads a def body: a
        // documentation form after the signature is metadata, not the value, and every body reader takes
        // `tail.get(1)` — so an un-stripped doc would be mis-read as the body. Done once here, in place
        // (no new nodes, ids unchanged), so every def kind is fixed uniformly (`strip_def_docs`). The
        // stripped text is CAPTURED (keyed by each def's signature occurrence) so the `DocOf`/`DocAt`
        // queries can still surface a definition's documentation after it leaves the body.
        let doc_by_sig = strip_def_docs(&mut ast);
        // Verification-annotation ENFORCEMENT (Inc-b (D), test-tier): rewrite every PLAIN `@requires` — `(@
        // (requires PRE) (def SIG BODY))` — so the def body becomes `(if PRE BODY (trap …))`, a body-entry
        // precondition check that TRAPS on violation. Runs BEFORE `proptest_gen::synthesize` (so a `@test`
        // over a `@requires`-constrained param wraps the already-checked body — the seam agreed with
        // v-property-testing) and BEFORE `strip_annotations` (so the `(@ (requires …) …)` wrapper is still
        // present to detect, and still recorded into `db.requires` afterward). A no-op for a program with no
        // plain `@requires`. (Universal `@ensures` enforcement — plain + `@test`-stacked via a shared helper
        // — is the immediately-following increment; this pass covers only plain `@requires` today.)
        crate::verify_enforce::enforce(&mut ast);
        // F1: for a `@test` whose parameter is a `(List <Int>)`, synthesize a nullary `Test.gen-int`-driven
        // generator WRAPPER (and drop `@test` from the original, which becomes the wrapper's callee), so
        // `cdz test` can property-test over a list rather than declining the compound param at the
        // boundary. Runs BEFORE `strip_annotations` (so the synthesized `@test` wrapper is hoisted as the
        // test) and `scan_top_level` (so its `(effect Test …)` + `(def …)` scan like hand-written source);
        // every node it appends is ordinary AST (`proptest_gen`, no name special-casing). A no-op for a
        // program with no compound-param `@test`.
        crate::proptest_gen::synthesize(&mut ast);
        // DATA-TYPE INVARIANT ESTABLISH (Part 1 + Part 2): for each type carrying `@invariant(PRED)`,
        // synthesize a typed checker `(def (__invariant_check_<T> (: it <T>)) <check-body>)` and, for a
        // single-payload newtype, a checked constructor `(def (__invariant_construct_<T> (: __inv_p U)) …)`,
        // both appended to the top-level items — auto-unwrapping a single-payload newtype so a bare scalar
        // predicate type-checks — so PRED is RESOLVED + TYPE-CHECKED with `it : T` in scope (a malformed
        // invariant is caught). BEFORE `strip_annotations` (wrapper present) + `scan_top_level`; all appended
        // nodes are ordinary AST. Returns the RAW-CONSTRUCTION node ids inside the construct-defs — the
        // construct sites `lower_sum_new` must EXEMPT from the establish divert (they are the checked
        // constructor's own `(T.V …)`, so re-routing them would recurse). No-op with no `@invariant`.
        let invariant_exempt_ctors = crate::invariant_establish::synthesize(&mut ast);
        // Normalize away a `(const BINDER)` PARAMETER wrapper on every `def`/`fn` signature BEFORE anything
        // reads a parameter: `const` marks a COMPILE-TIME parameter (`DESIGN-…-monomorphization` Addendum
        // 3), a declaration the specializer consumes — not part of the binder shape the resolver/typer
        // walk. Unwrap `(const (: d T))` → `(: d T)` in place (ids of the inner binder unchanged) and
        // RECORD the stripped param's name occurrence in `const_params`. Done here, like `strip_def_docs`,
        // so every downstream reader sees a PLAIN binder and const-ness lives only in the set.
        let const_params = strip_const_params(&mut ast);
        // Normalize away every known `(@ NAME (def …))` ANNOTATION on a definition BEFORE `scan_top_level`
        // — `inline-never`/`inline-always` (the emitter's inline policy, Addendum 4) and `test` (the
        // `cdz test` hoist marker). An annotation is a declaration a build phase consumes, not part of the
        // def shape every reader walks. Unwrap → `(def …)` in place and RECORD each def's BODY occ in the
        // matching set. Done here like `strip_def_docs`, so every downstream reader sees a plain `(def …)`
        // and the annotation lives only in the sets.
        let StrippedAnnotations {
            inline_never,
            inline_always,
            tests,
            tags,
            exhaustive,
            malformed_tags,
            requires,
            ensures,
            invariants,
            malformed_verify,
        } = strip_annotations(&mut ast);
        // `@param` SIDECAR — scan every `@param(widget: …) name : Type` site and GENERATE the `Param`
        // effect (one typed accessor op per param), appending it as a module member. Runs HERE, right
        // after `strip_annotations` and BEFORE `scan_top_level` below, so the generated `(effect Param
        // …)` is scanned as an ordinary effect declaration + synthesized like a hand-written one — and
        // BEFORE resolve, so a guest `(Param.width)` reference resolves against the generated accessor
        // (the generate-before-resolve ordering that dodges the Param-unbound circularity). The guest
        // reads the param's type from its EXPLICIT annotation, so no whole-program resolve is needed
        // here. An untyped `@param` (a B-invariant violation) needs no handling here — it is already
        // rejected upstream by `strip_annotations` (CDZ0201 "wraps no definition"), so this pass only
        // scans + generates the well-typed sites (see `param_sidecar` module docs).
        crate::param_sidecar::generate(&mut ast);
        // WORLD-IMPORT SIDECAR (no-redeclare surface): for an in-source top-level `(world …)` decl,
        // synthesize an `(effect <iface> (op …)…)` decl per import interface and append it as a module
        // member — HERE, BEFORE `scan_top_level`, so a guest that performs a NAMED world import
        // (`(host (iface) ((. iface op) …))`) resolves + types against the DERIVED effect with no
        // hand-written `(effect …)` decl (the generate-before-resolve ordering `param_sidecar` uses). The
        // synthesized decl is byte-shaped like a hand-written one, so downstream sees an ordinary effect.
        // A module with no in-source world (or no mappable import) is untouched.
        crate::wit_world::inject_world_import_effects(&mut ast);
        // WORLD-EXPORT PARAM DERIVATION (no-annotation boundary): the export-side mirror — for an in-source
        // `(world …)`, DERIVE each guest-export def's boundary param types from the matching world
        // guest-export member and inject them as `(: <param> <type>)` annotations, HERE (before
        // `scan_top_level`), so a reducer writes `(def (on-message msg) <step>)` with NO param annotation and
        // `msg` still types from the world's declared param (the one remaining boundary annotation removed;
        // nominal sums stay). An author-written annotation wins; a module with no in-source world (or no def
        // matching an export member) is untouched.
        crate::wit_world::derive_world_export_param_annotations(&mut ast);
        // The program's node count, captured BEFORE the prelude appends — the boundary between user
        // nodes (which the front-end's span table covers) and everything appended after. Ids `0..this`
        // are the user program; ids at/above are prelude or evaluator-synthesized.
        let user_node_count = ast.structure.len() as u32;
        // Install the prelude as ordinary AST nodes FIRST, so its records get `StructId`s (after the
        // program's — no program id shifts) and the parent index covers them too. A built-in module is
        // just a record in the arena; the prelude map is `name → its occurrence`.
        let mut prelude = crate::prelude::install(&mut ast);
        // The prelude's TYPE-CONSTRUCTOR / MODULE names (`Int`/`List`/`String`/…) — captured BEFORE the
        // built-in sums inject their DATA-CONSTRUCTOR names (`Some`/`None`/`Ok`/`Err`/…) into `prelude`
        // below. A user variant may legitimately shadow a data constructor (redeclaring `type Option =
        // Some(Int64) | None` rebinds bare `Some`/`None` to the user's ctor), but must NOT shadow a
        // type/module name — see the `variant_ctor_index` guard. Guarding against this pre-injection
        // snapshot, not the polluted map, is what keeps the two cases distinct.
        let prelude_type_module_names: crate::fxhash::FxHashSet<String> =
            prelude.keys().cloned().collect();
        let (defs, mut exports, mut type_decls, effect_decls, mut modules) = scan_top_level(&ast);
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
        // Synthesize each `(type …)` sum as a record — the built-in `Ast` record's ASSOCIATED FUNCTIONS
        // (`Ast.module` self-reflection; later `Ast.print`/`Ast.read`) are appended here too, because the
        // `Ast` `TypeDecl` CARRIES them (`TypeDecl.associated`, attached in `sums::prelude_decls`), so
        // `sum_record` appends them data-driven — no `Db::load` post-synthesis special-case (they live in
        // the prelude, like any prelude module's fields).
        crate::sums::synthesize(&mut ast, &mut type_decls);
        // Synthesize each `(effect …)` as a record (fields = operation values), the effect analogue of
        // the sum synthesis above — AFTER the scan (it reads `effect_decls`) and BEFORE the parent index
        // (which must index the synthesized nodes so a name inside a synthesized op type resolves).
        let mut effect_decls = effect_decls;
        crate::effects::synthesize(&mut ast, &mut effect_decls);
        // Synthesize each nested `(module …)` as a record (fields = its exported defs), the module analogue
        // of the sum/effect synthesis — AFTER the scan (it reads the module declarations) and BEFORE the
        // parent index (which must index the synthesized nodes so a member reference resolves by the walk).
        crate::modules::synthesize(&mut ast, &mut modules);
        // Desugar every CANONICAL handler `(handle E seed (bare-op-arm…) body)` — effect + seed promoted
        // into the head, arms written bare — into the INTERNAL `(handle seed ((. E op)-arm…) body)` the
        // resolver/effects/infer/lower/compile consume. Runs BEFORE the parent index so the rewritten
        // `(. E op)` projections resolve like hand-written member access. A handle already in internal
        // shape (4 children) is left untouched, so a hand-authored internal program still compiles.
        crate::effects::desugar_handles(&mut ast);
        // Reify every well-formed `(quote FORM)` into the `Ast` constructor application that BUILDS its
        // value (`(quote 42)` -> `(Ast.Int 42)`, `(quote (+ 1 2))` -> `(Ast.List (list …))`), so a quote
        // result and a hand-written `Ast.*` value are the SAME sum value (`metaprogramming.md` §Quote
        // Produces An AST Value). Purely structural — a quote is inert, its body never evaluated. Runs
        // BEFORE the parent index so the emitted `(. Ast …)`/`(list …)` nodes resolve like source. A
        // quote whose body mentions a leaf the `Ast` sum can't carry yet, or a wrong-arity `(quote …)`,
        // is left untouched for `resolve::resolve_quote` (a Todo decline / a CDZ0201, never a rewrite).
        // Returns the quote-PATTERN nodes with a NON-FINAL `,@` splice (ill-formed — a rest binds the
        // tail, meaningful only last), which `collect_faults` reports CDZ0221.
        let nonfinal_splice_patterns = crate::quote::reify_quotes(&mut ast);
        // Desugar every `(eval AST)` whose argument is a compile-time-visible `Ast` construction into the
        // SOURCE form that AST denotes — the inverse of the reification just run, so `(eval (quote (+ 1
        // 2)))` (now `(eval (Ast.List …))`) becomes `(+ 1 2)` and folds to `3` through the ordinary path
        // (`metaprogramming.md` §Eval Is Optional / §Compile-Time Evaluation Is One Tier). Runs AFTER
        // `reify_quotes` (so a quoted argument is already `Ast.*`) and BEFORE the parent index (so the
        // spliced-in source resolves like hand-written code). A non-constant/runtime AST argument is left
        // untouched for `resolve` to decline (the compiler does not execute a dynamically-built AST).
        crate::eval_ast::desugar_eval(&mut ast);
        // Expand every `(tagged-template <tag> (chunks …) (holes …))` (the reader's canonical form for the
        // ML surface `tag"…{expr}…"`) into the binding-dispatched application `(<tag> (list …) (list …))`.
        // The `tag` resolves in the ordinary environment to a compile-time `List String -> List Ast -> Ast`
        // function; applying it is an ordinary call the one-tier evaluator β-reduces (`eval::apply_lambda`),
        // splicing the returned `Ast` and folding it through the same path as generics / `(eval (quote …))`
        // — so an embedded DSL grows at the AST level through a library function, the reader stays fixed.
        // Runs AFTER quote/eval desugar and BEFORE the parent index (so the emitted call resolves like
        // source). A malformed node is left untouched for `resolve` (an unbound-name error on the head).
        crate::tagged_template::expand(&mut ast);
        // Normalize any type-suffixed numeric literal leaf (`100N`/`0.5R`) into the `(: <body>
        // BigInt|Rational)` annotation a suffix denotes. The reader desugars suffixes, so a `Leaf::Suffixed`
        // only survives from a codec source that preserves the kind (cadenza-ast's shared decode does, post
        // ast-consolidation) — this restores rcdzc's "compiler never sees a `Suffixed`" invariant so the
        // literal types via the ordinary annotation path. BEFORE the parent index, like the desugars above.
        crate::suffixed::normalize(&mut ast);
        // Every expansion above (handle desugar, quote reification, eval reconstruction) runs HERE at
        // load, BEFORE resolve/type/lower/compile — it produces ordinary AST the rest of the pipeline then
        // type-checks, capability-checks (the manifest is `collect_host_imports` over the EXPANDED AST, so
        // an expanded form reaches no capability the manifest does not enumerate), and determinism-checks
        // exactly as if that AST had been written directly. Expansion precedes and feeds the core guarantees.
        //= spec/capabilities/metaprogramming.md#expansion-precedes-and-feeds-the-core-guarantees
        //# The expanded representation MUST be subject to type checking exactly as if it had been written directly.
        //= spec/capabilities/metaprogramming.md#expansion-precedes-and-feeds-the-core-guarantees
        //# The expanded representation MUST be subject to capability checking exactly as if it had been written directly.
        //= spec/capabilities/metaprogramming.md#expansion-precedes-and-feeds-the-core-guarantees
        //# The expanded representation MUST be subject to the determinism guarantees exactly as if it had been written directly.
        //= spec/capabilities/metaprogramming.md#expansion-precedes-and-feeds-the-core-guarantees
        //# A macro MUST NOT be able to produce an expanded representation that reaches a capability the program's manifest does not enumerate.
        // ACCUMULATOR INTRODUCTION: rewrite a linear NON-tail recursion (`f n = if base 0 (+ n (f (- n
        // 1)))`) into a tail-recursive accumulator def (which `select`'s loop transform then compiles to a
        // constant-stack `loop`). Synthesizes a fresh accumulator def and re-seeds the original — appending
        // to `defs` and the arena HERE, before the parent index / `def_by_name` are built, so the new def
        // and its self-reference resolve like any hand-written def. A def that does not match is untouched.
        let mut defs = defs;
        let accum_links = crate::accum::introduce(&mut ast, &mut defs);
        // SET.OF OVER A RUNTIME LIST: rewrite `(Set.of xs)` whose `xs` is not a `(list …)` literal into a
        // synthesized recursive fold (`__set_of_rt$ xs 0 (Set.of (list))`), so a set builds from a
        // runtime list (a `Set.to-list` result, a `List.concat`, a param/recursively-built list) instead
        // of declining. Appends one generic fold def + rewrites the calls HERE, before the parent index /
        // `def_by_name`, so the synthesis resolves + type-checks like hand-written source. Hash-neutral
        // (reuses `List.len`/`List.at`/`Set.insert`/empty `Set.of`). See `set_of_runtime` for the
        // one-element-type-per-program limitation (the recursive-generic Set-op grounding tie).
        crate::set_of_runtime::introduce(&mut ast, &mut defs);
        // BYTES.OF OVER A RUNTIME LIST: the `Bytes` twin of `set_of_runtime` — rewrite `(Bytes.of xs)`
        // whose `xs` is not a `(list …)` literal into a synthesized recursive fold (`__bytes_of_rt$ xs 0
        // (Bytes.of (list))`) that appends each byte via `Bytes.concat`, so a byte sequence builds from a
        // runtime `(List UInt8)` (a `List.concat`, a param/recursively-built list) instead of declining.
        // Hash-neutral (reuses `List.len`/`List.at`/`Bytes.concat`/empty `Bytes.of`). `Bytes.of` is
        // monomorphic (`(List UInt8) → Bytes`), so unlike `set_of_runtime` there is no multi-type limit.
        crate::bytes_of_runtime::introduce(&mut ast, &mut defs);
        // DESTRUCTURING PARAMETERS: rewrite a `def` whose parameter is a tuple PATTERN (`(def (f (tuple a
        // b)) …)`) into a fresh whole-value parameter plus a destructuring `let` over the body (`(def (f
        // p$0) (let (((tuple a b) p$0)) …))`) — so a body reference to `a`/`b` resolves through the `let`
        // machinery the binding-pattern `let` case already realizes. Runs HERE (after accum, before the
        // parent index / `def_by_name`) so the rewritten def resolves like a hand-written one.
        crate::binding_params::lower(&mut ast, &mut defs);
        // Register each MODULE MEMBER FUNCTION as an INTERNAL def, so a RECURSIVE call to it lowers to a
        // standalone `Core::Call` (a stable `db.defs` index) instead of declining. Runs HERE — after the
        // top-level transforms (accum/binding-params reshape the TOP-LEVEL defs; a module member compiles
        // as an ordinary recursive call), before the parent index / `def_by_body` (which must index the
        // member's body → its def) and before `def_name_index` (an internal def is EXCLUDED from it — its
        // name resolves by scope, `resolve::module_sibling_binds`, per the no-keys-outside-the-prelude rule).
        crate::modules::register_callable(&ast, &modules, &mut defs);
        // Register each DO-LOCAL FUNCTION declaration the same way — the do-block analogue: a do-local
        // recursive/mutually-recursive `(def (f p…) …)` resolves by scope (`resolve::do_local_binds`) but
        // still needs a `db.defs` index to lower its call to a `Core::Call`. Same INTERNAL discipline
        // (excluded from `def_name_index`; not an export/warning target).
        crate::modules::register_do_local_callables(&ast, &mut defs);
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
        // The set of nested-module SYNTHESIZED RECORD ids — each is a scope (its members are mutually
        // visible), so the scope-skip index must land the walk on it (see `is_binding_candidate`). Built
        // once here from `modules` (all `synth` are `Some` post-synthesis); an O(1) membership probe.
        let module_records: crate::fxhash::FxHashSet<StructId> =
            modules.iter().filter_map(|m| m.synth).collect();
        // Per-node lexical-scope skip pointer (nearest binding-candidate ancestor + entry child) so the
        // scope walk hops over non-binding forms — O(1) per binder instead of O(nesting depth).
        let mut scope_skip = build_scope_skip(&ast, &parent, &module_records);
        // The load-time boundary: every id currently in the vector has a GENUINE computed entry. A synth
        // node appended later is uncovered until explicitly seeded (see `scope_skip_seeded`).
        let scope_skip_load_boundary = scope_skip.len();
        // CHAIN each module's SYNTHESIZED RECORD out to the enclosing scope of its DECLARATION occurrence.
        // A member body's scope walk ascends body → the module's synth `fn` → its synth RECORD (where Case
        // R / `module_sibling_binds` resolves a SIBLING MEMBER of the SAME module). But the record is a
        // load-appended node whose own parent chain dead-ends, so without this the walk STOPS there — a
        // member body could not see a name bound in the module's ENCLOSING scope (a SIBLING MODULE `lib`
        // declared beside `(module app …)` in the same `do`, or an enclosing `let`/`def` binding). Point
        // the record's skip at whatever the module's DECLARATION `occ` sees (`scope_skip[occ]` — the
        // nearest binding candidate above the `(module …)` form), so after the record's own members the
        // walk continues into the module's lexical context, exactly as a top-level def's body does. The
        // record stays a binding candidate itself (its members resolve when the walk LANDS on it via
        // `binder_in`'s Case R); this only extends where it goes NEXT. Definition-site correct: it chains
        // to where the module was WRITTEN, so `lib`'s own literals keep `lib`'s scope, not `app`'s.
        for m in &modules {
            if let (Some(record), occ) = (m.synth, m.occ)
                && (record.0 as usize) < scope_skip.len()
            {
                scope_skip[record.0 as usize] = scope_skip[occ.0 as usize];
            }
        }
        // Index each SCOPE FORM's parameter binders by name (last-wins), so `binder_in`'s per-reference
        // "does this scope declare `name`?" probe is O(1) rather than an O(params) signature scan.
        let scope_binders = build_scope_binders(&ast);
        // Over-approximate each MATCH ARM's pattern name set, so a `binder_in` hop over an arm that cannot
        // bind `name` (the common global/prelude/ctor reference in a deeply-nested match) fast-rejects in
        // O(1) instead of running the ~20-case arm cascade — the O(depth)-per-reference resolve pole.
        let arm_pattern_names = build_arm_pattern_names(&ast, &parent);
        // Index each let bindings-list's bare-name binders by name (ascending positions + value occ), so
        // `last_binder_named`'s per-reference reverse scan is O(log N) rather than an O(N) prefix walk — a
        // wide accumulation `let` was O(N²). Destructuring-pattern lists fall back to the linear scan.
        let let_binder_index = build_let_binder_index(&ast);
        let do_binder_index = build_do_binder_index(&ast);
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
            // An INTERNAL def (a module-member callable) is NOT name-resolvable — its name resolves by
            // lexical scope (`resolve::module_sibling_binds`), never a global lookup — so keep it OUT of
            // `def_name_index`. (`def_by_body` above DOES index it, so `callee_def_index` finds it.)
            if !d.internal {
                def_by_name.entry(d.name.clone()).or_insert(i);
            }
        }
        // Index each user-sum VARIANT NAME → its constructor occurrence (first-declared wins), from the
        // `ctor` synthesize cached on each variant. `variant_ctor_by_name` reads this O(1) instead of an
        // O(variants) scan per bare-variant reference — an N-arm match over an N-variant sum was O(N²).
        let mut variant_ctor_index: crate::fxhash::FxHashMap<String, StructId> =
            crate::fxhash::FxHashMap::default();
        // The companion index of the variants the prelude-collision guard below SKIPS — a variant whose
        // name shadows a prelude TYPE/MODULE name (`Int`/`List`/…). `resolve_name` reaches these ONLY in
        // application-HEAD position on a user node (a genuine `(Int 42)` construct), where the user's
        // variant shadows the colliding prelude name (operator ruling). First-declared wins.
        let mut prelude_colliding_variant_ctor_index: crate::fxhash::FxHashMap<String, StructId> =
            crate::fxhash::FxHashMap::default();
        // Index each sum TYPE NAME → its synthesized record (first-declared wins) — `type_decl_by_name`'s
        // O(1) lookup, replacing an O(types) `find` per name resolution (O(N²) for N sum types).
        let mut type_decl_index: crate::fxhash::FxHashMap<String, StructId> =
            crate::fxhash::FxHashMap::default();
        // Index each SAME-NAME NEWTYPE's name → its ctor occurrence (see the loop body) — O(1)
        // `same_name_newtype_ctor`, replacing an O(types) `find` per type-name resolution.
        let mut same_name_newtype_ctor_index: crate::fxhash::FxHashMap<String, StructId> =
            crate::fxhash::FxHashMap::default();
        let mut same_name_monomorphic_ctor_index: crate::fxhash::FxHashSet<String> =
            crate::fxhash::FxHashSet::default();
        // The boundary between USER `(type …)` declarations and the appended BUILT-IN prelude sums (the
        // last `prelude_sum_count`, e.g. `Ast`/`Option`/`Result`). Only a USER declaration's colliding
        // variant shadows the prelude in construct position (the operator ruling is about a program-defined
        // name); the built-in `Ast`'s `Int`/`Name`/`List` variants MUST stay qualified-only (`Ast.Int`) —
        // indexing them would make a bare `(Int 42)` width-type ctor resolve to `Ast.Int`, re-opening the
        // `9f326a2d` global corruption via the built-in.
        let first_prelude_sum = type_decls.len() - prelude_sum_count;
        for (di, decl) in type_decls.iter().enumerate() {
            let is_user_decl = di < first_prelude_sum;
            for v in &decl.variants {
                // A variant's bare name resolves BEFORE the prelude (`resolve` step 3c precedes step 4), so
                // a variant whose name COLLIDES with a built-in prelude TYPE-CONSTRUCTOR / MODULE name
                // (`Int`/`List`/`Name`) would SHADOW it, breaking that name everywhere it is used as a
                // type/module (a payload `(Int Int64)`, an annotation `(: x Int64)` whose reduction touches
                // `Int`). Such a colliding variant is reached ONLY qualified — `(. T Int)` / the built-in
                // `(. Ast Int)` — via the sum RECORD's field, never the bare-name index; so DON'T index it.
                // Guard against the PRE-INJECTION snapshot (`prelude_type_module_names`), NOT the current
                // `prelude` map: by now the built-in sums have injected their DATA-CONSTRUCTOR names
                // (`Some`/`None`/`Ok`/`Err`/…) into `prelude`, and a user variant MAY shadow those — a
                // redeclared `(type Option (Some Int64) None)` rebinds bare `Some`/`None` to the user's ctor
                // (the common case). Checking the polluted map would wrongly skip those, so bare `Some`/`None`
                // would fall through to the built-in generic Option — a silent miscompile.
                if prelude_type_module_names.contains(&v.name) {
                    // Skipped from the bare index (so bare `Int` stays the width ctor everywhere it means
                    // the width type) but — for a USER declaration only — recorded in the companion so a
                    // HEAD-position user construct `(Int 42)` reaches the local variant (the construct-half
                    // of the shadowing). A BUILT-IN prelude sum's colliding variant (`Ast.Int`) is NOT
                    // recorded: it stays qualified-only, else bare `(Int 42)` would resolve to `Ast.Int`.
                    if is_user_decl && let Some(ctor) = v.ctor {
                        prelude_colliding_variant_ctor_index
                            .entry(v.name.clone())
                            .or_insert(ctor);
                    }
                    continue;
                }
                if let Some(ctor) = v.ctor {
                    variant_ctor_index.entry(v.name.clone()).or_insert(ctor);
                }
            }
            if let Some(synth) = decl.synth {
                type_decl_index.entry(decl.name.clone()).or_insert(synth);
            }
            // A SAME-NAME CONSTRUCTOR — a sum with a VARIANT whose name shares the type's name
            // (`(type UserId (UserId Int64))`, or the first variant of a multi-variant `(type N (N Int64)
            // (J Int64))`) — indexed name → its ctor occurrence, for `same_name_newtype_ctor`'s O(1)
            // lookup. The ONE name `N`/`UserId` must mean the CONSTRUCTOR in application/pattern-HEAD
            // position (`(N a)`, `(N v)`) while `type_decl_by_name` keeps it the TYPE everywhere else
            // (`(: x N)`): resolve picks by POSITION, never by variant COUNT — a multi-variant sum whose
            // first variant shares the type name is the same head-position collision as a single-variant
            // newtype (the breaker's `adv-same-name-ctor-hijacked-by-type` case-2). NOT restricted to
            // one-variant sums: the extra variants (`J`) are irrelevant to whether the SAME-NAME variant's
            // bare application resolves to the ctor. First matching variant wins (a sum has at most one
            // variant named exactly its own type name — variant names are unique within a decl). First-
            // declared type wins across sums, matching the prior `find`'s first-hit.
            if let Some(ctor) = decl
                .variants
                .iter()
                .find(|v| v.name == decl.name)
                .and_then(|v| v.ctor)
            {
                same_name_newtype_ctor_index
                    .entry(decl.name.clone())
                    .or_insert(ctor);
                // MONOMORPHIC same-name sum (no type params) — record the name so `resolve_name` may fire
                // the head-position ctor rule on a SYNTH node too (a β-copied `(Meters a)` inlined at a call
                // site). Safe only for a monomorphic sum: a GENERIC one has a `sum_applied` synth type-expr
                // `(Box a)` in head position that MUST stay the type, and only `is_user_node` tells that
                // synth from a value-construct copy — but a monomorphic sum has NO such synth. (FACE B of the
                // breaker's `adv-same-name-ctor-hijacked-by-type`.)
                if decl.params.is_empty() {
                    same_name_monomorphic_ctor_index.insert(decl.name.clone());
                }
            }
        }
        // Sum DECLARATION OCCURRENCE → its index in `type_decls` — `type_decl_by_occ`'s O(1) reverse (was
        // a linear scan; the rust-backend recursive-sum boxing walk calls it per sum-graph node → O(N³)
        // for N mutually-recursive sums). An occ is unique to one declaration, so no collision.
        let mut type_decl_by_occ_index: crate::fxhash::FxHashMap<StructId, usize> =
            crate::fxhash::FxHashMap::default();
        for (i, decl) in type_decls.iter().enumerate() {
            type_decl_by_occ_index.insert(decl.occ, i);
        }
        // Effect NAME → synthesized record (first-declared wins) — `effect_decl_by_name`'s O(1) lookup.
        let mut effect_decl_index: crate::fxhash::FxHashMap<String, StructId> =
            crate::fxhash::FxHashMap::default();
        for e in &effect_decls {
            if let Some(synth) = e.synth {
                effect_decl_index.entry(e.name.clone()).or_insert(synth);
            }
        }
        // Derive the file-scoped resolution table from the package linkage (`Some` only for a >1-file
        // package). For each file: its own top-level VALUE defs (a def whose signature occurrence falls
        // in that file's `StructId` range), plus its imports (each local name → the exporting file's
        // def of that name). A single-file compile carries no linkage, so this stays `None` and every
        // lookup falls through to the flat `def_name_index` — byte-identical to before.
        // The per-file demux ranges — captured BEFORE `linkage` moves into `file_scope` below, so the
        // root-scope pragma harvest can scope a `(pragma …)` to the FILE that declares it (a file IS a
        // module; a default-fraction/integer directive is module-scoped and MUST NOT leak across an import
        // boundary into another file's literals). `None` for a single-file compile (no cross-file scope to
        // worry about — every node is "the same file").
        let link_files: Option<Vec<crate::link::FileSpan>> =
            linkage.as_ref().map(|lk| lk.files.clone());
        let file_scope = linkage.map(|lk| {
            build_file_scope(
                &lk,
                &defs,
                &def_by_name,
                &type_decls,
                &prelude_type_module_names,
            )
        });
        // RE-BIND each export to the def it names WITHIN ITS OWN FILE — the file-scoped analogue of the
        // flat first-wins `def_of_name` that `scan_top_level` used. In a linked multi-file package the
        // flat bind lets a same-named def in a sibling file (spliced earlier, even a PRIVATE one) hijack
        // the entry's exported name, so the component runs the wrong file's code and returns its value
        // (silent wrong-program — `package-export-boundary-binds-flat-def-by-name-not-per-file`). The
        // internal CALL path already resolves per-file (`file_scoped_def`); this is the one boundary site
        // that missed it. Resolve each export in the file that WROTE its `(export …)` clause (via the
        // name atom's demux range) — never a sibling's scope (`DESIGN-package-linking.md` §4). A
        // single-file compile has no `file_scope`, so this is skipped and the flat bind stands (unchanged).
        // If the exporting file's own scope has no such value def (a type-handle export, or a name only
        // present in a sibling), leave the scan's binding — that path (type export / diagnostics) is
        // unaffected by the collision this fixes.
        if let Some(fs) = file_scope.as_ref() {
            for e in &mut exports {
                if let Some(idx) = fs.export_def(e.name_occ, &e.name) {
                    e.def = Some(idx);
                }
            }
        }
        // Scan the program's `(Unit.define …)` forms BEFORE `ast` moves into the db (a raw-occurrence
        // store the units layer reduces on demand).
        let unit_defines = scan_unit_defines(&ast);
        // Scan `(bind Effect "iface")` top-level directives → the effect→peer-contract default map (U2).
        let effect_bindings = scan_effect_bindings(&ast);
        // Map each bare integer LITERAL written inside a `(pragma default-integer <T>)` module to its
        // default type-expr — an ARENA walk of each pragma-module's member subtrees (before `ast` moves,
        // before any β-copy), so a literal's default survives inlining that reparents it (the parent-walk
        // a naive lookup would use is unusable post-copy). See `default_int_literals`.
        let mut default_int_literals = crate::fxhash::FxHashMap::default();
        for m in &modules {
            if let Some(ty_expr) = m.default_int {
                collect_default_int_literals(&ast, m.occ, ty_expr, &mut default_int_literals);
            }
        }
        // Map each bare NUMERIC literal (integer OR decimal) written inside a `(pragma default-fraction
        // <T>)` module to its rational default type-expr — the exactness analogue of the integer map.
        let mut default_fraction_literals = crate::fxhash::FxHashMap::default();
        for m in &modules {
            if let Some(ty_expr) = m.default_fraction {
                collect_default_fraction_literals(
                    &ast,
                    m.occ,
                    ty_expr,
                    &mut default_fraction_literals,
                );
            }
        }
        // Mark each bare, un-suffixed integer literal written as the direct argument of a constructor whose
        // declared payload type is `BigInt` — so it GROUNDS to `Ty::BigInt` instead of the Int64 default
        // (`numeric-model.md`: an integer literal grounds to BigInt losslessly; an explicit context takes
        // precedence over the declared default — a grounding, not a promotion). Load-time (not check-time)
        // because `infer` memoizes the arg's Int64 type before the ctor-payload unify site could intervene.
        let mut bigint_ctor_arg_literals = crate::fxhash::FxHashSet::default();
        collect_bigint_ctor_arg_literals(&ast, &type_decls, &mut bigint_ctor_arg_literals);
        // Map each bare DECIMAL literal written inside a `(pragma default-float <T>)` module to its default
        // float type-expr — the floating-point analogue of the integer map (decimals only; an integer
        // literal keeps its integer default).
        let mut default_float_literals = crate::fxhash::FxHashMap::default();
        for m in &modules {
            if let Some(ty_expr) = m.default_float {
                collect_default_float_literals(&ast, m.occ, ty_expr, &mut default_float_literals);
            }
        }
        // Map each unqualified `+`/`-`/`*` arithmetic node written inside a `(pragma overflow …)` module to
        // its governing OverflowSpec — an ARENA walk of each pragma-module's member subtrees (before `ast`
        // moves, before any β-copy), keyed by the original operator node so an inlined body's op still finds
        // its policy. The signed-vs-unsigned SELECTION is deferred to `infer` (post-monomorphization). See
        // `overflow_specs`.
        let mut overflow_specs = crate::fxhash::FxHashMap::default();
        for m in &modules {
            if let Some(spec) = m.overflow {
                collect_overflow_modes(&ast, m.occ, spec, &mut overflow_specs);
            }
        }
        // ROOT/FILE-TOP scope. A `(pragma <key> <T>)` written as a TOP-LEVEL item — the root `(do …)`'s
        // own child or a bare-file form (`read_all` wraps top-level forms in a synthetic `(do …)`), NOT
        // inside a nested `(module NAME …)` — declares the default for the WHOLE root module, exactly as
        // a nested module's pragma does for its members. numeric-model.md §"A Module May Declare Its
        // Default … Literal Type" imposes no do-nesting requirement, and a file IS a module; the impl
        // previously honored only nested-module pragmas (CDZ0601 at the root). Mark each root-scope
        // `(def …)` body's literals with the root pragma's `<T>`, the file-top analogue of the per-module
        // harvest. Definition-site scoped like the module case (`mark_*_literals` does not descend a nested
        // `(module …)`, so an inner module keeps its own default). Scanned from the root's top items.
        // The file a top-level node belongs to (in a linked package), or `None` for a single-file compile
        // (then every node is "the same file" — the leak cannot occur). Used to keep a file-top pragma's
        // effect INSIDE the file that declares it: a `(pragma default-fraction …)` in `app.cdz` must NOT
        // ground the literals of an IMPORTED `lib.cdz` (numeric-model.md §"…WITHIN THAT MODULE"; a file is
        // a module). The linked arena splices every file's items under ONE synthetic `(do …)` root, so
        // `top_items` returns items from ALL files — the pragma's SIBLINGS include imported files' defs
        // unless filtered by file.
        // BINARY SEARCH the demux, not a linear `position(contains)`: the harvest below calls this inside a
        // nested `top_items × top_items` loop (once per pragma × per sibling), so a linear scan is
        // O(pragmas × siblings × files) on a multi-file package (Copilot perf note, PR #523). The files are
        // spliced sequentially, so `link_files` is SORTED by `struct_base` with non-overlapping ranges —
        // `partition_point` finds the last file whose base is ≤ `id` in O(log files), then a single
        // `contains` confirms `id` is within that file's range (a node in NO file — the synth `(do …)` root
        // — falls between/after ranges and fails the check). Same result as the linear scan, log-time.
        let file_of = |id: StructId| -> Option<usize> {
            let fs = link_files.as_ref()?;
            let idx = fs
                .partition_point(|f| f.struct_base <= id.0)
                .checked_sub(1)?;
            fs.get(idx).filter(|f| f.contains(id)).map(|_| idx)
        };
        for &item in &top_items(&ast) {
            let Some(ptail) = ast.as_form(item, "pragma") else {
                continue;
            };
            let (Some(key), Some(&ty_expr)) =
                (ptail.first().and_then(|&k| ast.as_name(k)), ptail.get(1))
            else {
                continue;
            };
            // The file that DECLARES this pragma; only sibling defs in the SAME file are marked (a
            // single-file compile has `link_files == None` → `pragma_file == None`, and the same-file test
            // below is trivially true, so behavior is unchanged there).
            let pragma_file = file_of(item);
            let same_file = |sib: StructId| link_files.is_none() || file_of(sib) == pragma_file;
            // `overflow`'s second element is a `(signed …)` sub-form, not a type-expr — parse the whole
            // spec once from the pragma form (the default-* keys use `ty_expr` directly).
            let overflow_spec = (key == "overflow")
                .then(|| parse_overflow_spec(&ast, item))
                .flatten();
            for &sib in &top_items(&ast) {
                if !same_file(sib) {
                    continue;
                }
                let Some(def_tail) = ast.as_form(sib, "def") else {
                    continue;
                };
                let Some(&def_body) = def_tail.get(1) else {
                    continue;
                };
                match key {
                    "default-integer" => {
                        mark_int_literals(&ast, def_body, ty_expr, &mut default_int_literals)
                    }
                    "default-fraction" => mark_numeric_literals(
                        &ast,
                        def_body,
                        ty_expr,
                        &mut default_fraction_literals,
                    ),
                    "default-float" => {
                        mark_float_literals(&ast, def_body, ty_expr, &mut default_float_literals)
                    }
                    "overflow" => {
                        if let Some(spec) = overflow_spec {
                            mark_overflow_nodes(&ast, def_body, spec, &mut overflow_specs);
                        }
                    }
                    _ => {}
                }
            }
        }
        // Every node inside a TYPE-EXPRESSION subtree — so the construct-position shadow (`resolve_name`
        // step 3d) fires only in genuine VALUE position and leaves a `(List Ast)` payload / annotation type
        // as the List TYPE constructor. Roots: every `(: e T)` annotation's type slot, every variant
        // payload, and every effect-op arrow. Marking the whole subtree of each root is a safe superset (a
        // value construct never sits inside a type expression). See `type_expr_nodes`.
        let mut type_expr_nodes = crate::fxhash::FxHashSet::default();
        // `(: e T)` annotation type slots — anywhere in the program (param/value/return annotations). One
        // arena walk marks each `:` form's SECOND tail element (the type) and its descendants.
        for id in 0..ast.structure.len() as u32 {
            let id = StructId(id);
            if let Some(t) = ast.as_form(id, ":")
                && let Some(&ty_slot) = t.get(1)
            {
                mark_subtree(&ast, ty_slot, &mut type_expr_nodes);
            }
        }
        // Variant payload type expressions (`(List Ast)` in `(type Ast … (List (List Ast)))`) — not under a
        // `:`, so seeded from the scanned decls. Prelude sums are included harmlessly (their payloads are
        // type exprs too); the shadow never reaches them anyway (their variants aren't in the companion).
        for decl in &type_decls {
            for v in &decl.variants {
                for &p in &v.payloads {
                    mark_subtree(&ast, p, &mut type_expr_nodes);
                }
            }
        }
        // Effect-op arrows (`(op f (-> (List a) Bool))`) — the op's whole type expression.
        for e in &effect_decls {
            for op in &e.ops {
                if let Some(ty) = op.ty {
                    mark_subtree(&ast, ty, &mut type_expr_nodes);
                }
            }
        }
        let mut db = Db {
            ast,
            defs,
            exports,
            type_decls,
            effect_decls,
            component_name: None,
            emit_debug: false,
            source_snapshots: Vec::new(),
            wit_world: None,
            effect_bindings,
            modules,
            newtype_inner: crate::fxhash::FxHashMap::default(),
            default_int_literals,
            default_fraction_literals,
            bigint_ctor_arg_literals,
            resume_escape_operands: None,
            default_float_literals,
            overflow_specs,
            // No manifest global default at load; the manifest ingestion (v-cdz-crate-split) sets it when a
            // `Project.cdz` declares `overflow-signed`/`overflow-unsigned`. Default → module-pragma-or-trap.
            global_overflow: OverflowSpec::default(),
            enum_disc: crate::fxhash::FxHashSet::default(),
            parent,
            child_ix,
            scope_skip,
            scope_skip_load_boundary,
            scope_skip_seeded: crate::fxhash::FxHashSet::default(),
            scope_skip_force_uncovered: crate::fxhash::FxHashSet::default(),
            module_records,
            def_by_body,
            doc_by_sig,
            def_by_sig: None,
            def_by_ident: None,
            def_name_index: def_by_name,
            variant_ctor_index,
            prelude_colliding_variant_ctor_index,
            type_expr_nodes,
            type_decl_index,
            same_name_newtype_ctor_index,
            same_name_monomorphic_ctor_index,
            type_decl_by_occ_index,
            effect_decl_index,
            scope_binders,
            arm_pattern_names,
            let_binder_index,
            do_binder_index,
            prelude,
            unit_families: crate::prelude::unit_families(),
            unit_defines,
            file_scope,
            user_node_count,
            nonfinal_splice_patterns,
            reduce_depth: 0,
            reduce_nodes: 0,
            structural_reductions: 0,
            fold_type_fault: None,
            descent_depth: 0,
            walk_depth: 0,
            callee_visited: crate::fxhash::FxHashSet::default(),
            closure_code_visited: crate::fxhash::FxHashSet::default(),
            closure_sig_visited: crate::fxhash::FxHashSet::default(),
            host_import_visited: crate::fxhash::FxHashSet::default(),
            host_arg_string_visited: crate::fxhash::FxHashSet::default(),
            collect_limited: false,
            reached_visited: crate::fxhash::FxHashSet::default(),
            reached_clipped: false,
            synth_name_origin: crate::fxhash::FxHashMap::default(),
            build_cache: crate::fxhash::FxHashMap::default(),
            recursive: crate::fxhash::FxHashMap::default(),
            reaches_host_call: crate::fxhash::FxHashMap::default(),
            escape_verdict_memo: crate::fxhash::FxHashMap::default(),
            payload_sites_memo: crate::fxhash::FxHashMap::default(),
            captured_occ_memo: crate::fxhash::FxHashMap::default(),
            is_cse_shareable_memo: crate::fxhash::FxHashMap::default(),
            ty_has_free_var: crate::fxhash::FxHashMap::default(),
            callee_edges: crate::fxhash::FxHashMap::default(),
            scheme_cache: crate::fxhash::FxHashMap::default(),
            record_field_index: crate::fxhash::FxHashMap::default(),
            simple_list_binders: std::cell::RefCell::new(crate::fxhash::FxHashMap::default()),
            let_region_uses: std::cell::RefCell::new(crate::fxhash::FxHashMap::default()),
            mutual_loop_cache: crate::fxhash::FxHashMap::default(),
            reduce_cache: crate::fxhash::FxHashMap::default(),
            collect_cache: crate::fxhash::FxHashMap::default(),
            delegated_effects: None,
            rec_visited: crate::fxhash::FxHashSet::default(),
            rec_worklist: Vec::new(),
            kept_bindings: crate::fxhash::FxHashSet::default(),
            strict_force_eval: crate::fxhash::FxHashSet::default(),
            lifted: Vec::new(),
            lifted_by_body: crate::fxhash::FxHashMap::default(),
            sum_reachable: crate::fxhash::FxHashMap::default(),
            sum_out_edges: crate::fxhash::FxHashMap::default(),
            suggest_pool: [None, None, None],
            suggest_pool_winner: crate::fxhash::FxHashMap::default(),
            variant_suggest_winner: crate::fxhash::FxHashMap::default(),
            variant_closest_matches: crate::fxhash::FxHashMap::default(),
            no_field_suggestion: crate::fxhash::FxHashMap::default(),
            captured_ref: crate::fxhash::FxHashMap::default(),
            resolved_subtrees: crate::fxhash::FxHashSet::default(),
            def_schemes: crate::fxhash::FxHashMap::default(),
            param_types: crate::fxhash::FxHashMap::default(),
            scheme_rigid_vars: None,
            arrow_lambdas_in_progress: crate::fxhash::FxHashSet::default(),
            solving_params: crate::fxhash::FxHashSet::default(),
            scc_result_typing: crate::fxhash::FxHashSet::default(),
            solving_schemes: crate::fxhash::FxHashSet::default(),
            seed_transitive: crate::fxhash::FxHashSet::default(),
            reduction_root: None,
            call_sites_by_callee: None,
            emit_shared: crate::fxhash::FxHashMap::default(),
            resolved: Column::new(),
            #[cfg(test)]
            resolved_of_calls: 0,
            #[cfg(test)]
            cse_partition_core_eq_calls: 0,
            #[cfg(test)]
            value_range_uncached_calls: 0,
            #[cfg(test)]
            param_apply_extra_handled_calls: 0,
            #[cfg(test)]
            is_cse_shareable_uncached_calls: 0,
            types: Column::new(),
            typeval: Column::new(),
            typeval_memo_live: false,
            core: Column::new(),
            core_override: crate::fxhash::FxHashMap::default(),
            local_lift_captures: crate::fxhash::FxHashMap::default(),
            effect_specializations: crate::fxhash::FxHashMap::default(),
            multivalue_specs: std::collections::HashSet::new(),
            effect_spec_captures: std::collections::HashMap::new(),
            force_multivalue: std::collections::HashSet::new(),
            handler_region_nodes: std::collections::HashSet::new(),
            group_multivalue_bodies: std::collections::HashSet::new(),
            mutual_scc: crate::fxhash::FxHashMap::default(),
            subtree_performs_cache: crate::fxhash::FxHashMap::default(),
            reduced_callable_walked: crate::fxhash::FxHashSet::default(),
            type_specializations: crate::fxhash::FxHashMap::default(),
            const_repass_fp: crate::fxhash::FxHashMap::default(),
            last_const_closure_fp: crate::fxhash::FxHashMap::default(),
            instantiations: Vec::new(),
            inlined: crate::fxhash::FxHashSet::default(),
            called: crate::fxhash::FxHashSet::default(),
            transformed: accum_links.into_iter().collect(),
            const_params,
            invariant_exempt_ctors,
            inline_never,
            inline_always,
            tests,
            tags,
            exhaustive,
            malformed_tags,
            requires,
            ensures,
            invariants,
            malformed_verify,
            range_refinements: Vec::new(),
            value_range_memo: crate::fxhash::FxHashMap::default(),
        };
        // NEWTYPE ERASURE: materialize, once, which declared sums are erasable NEWTYPES and their
        // underlying structural type. `decode_ty` consults this to normalize a `Ty::Sum` naming an
        // erasable decl into a `Ty::Nominal`, so every type reader agrees on the representation. Run AFTER
        // the `Db` is built (the predicate uses `typeval_of` over the synthesized ctor payloads) and
        // BEFORE any type/scheme is computed (so nothing caches a pre-normalization `Sum`). Only USER +
        // prelude sum decls are probed; a non-newtype decl contributes no entry (stays boxed).
        let decls: Vec<StructId> = db.type_decls.iter().map(|t| t.occ).collect();
        for decl in decls {
            if let Some(inner) = crate::infer::newtype_underlying(&mut db, decl) {
                db.newtype_inner.insert(decl, inner);
            }
        }
        // RE-NORMALIZE the stored templates against the NOW-COMPLETE `newtype_inner`. The loop above
        // decodes each decl's template in `type_decls` order, so a decl that references ANOTHER erasable
        // newtype declared LATER (in an imported module, `(type Note (Note Pitch ..))` decoded before
        // `Pitch` is keyed — cross-module linking splices imports in file order) baked a raw `Ty::Sum{that
        // decl}` into its template: `newtype_underlying(Note)` decoded the `Pitch` field through
        // `typeval_of`, which `normalize_sum`'d it while `Pitch` was not yet a key → boxed `Sum`, not the
        // erased `Nominal`. Left raw, a projection of the outer newtype reads a `Ty::Sum` for that field,
        // and the backend treats it as a heap handle (no unbox) though the value is the erased inner — an
        // invalid-wasm miscompile (9939). `newtype_inner` exists so EVERY reader agrees on the
        // representation, so the templates themselves must agree: walk each and replace an embedded
        // now-keyed `Ty::Sum` with its `normalize_sum` (`Nominal`). One pass suffices — the map is complete,
        // so a single walk resolves every cross-decl reference. Computed from the originals then written
        // back together, so each `normalize_sum` reads the pre-rewrite template (a recursive newtype's
        // self-referential `Sum` back-edge normalizes to the SAME one-level-unfold `Nominal{inner:
        // template}` shape query-time `normalize_sum` already produces — finite, not re-expanded).
        // ITERATE TO A FIXPOINT (PR#659 / Copilot): a single pass is NOT enough for a TRANSITIVE chain in
        // reversed dependency order. `normalize_embedded_sums` stops at a produced `Ty::Nominal` WITHOUT
        // descending its `inner` (a recursive newtype's `Sum` back-edge would loop); it relies on
        // `normalize_sum` re-deriving that `inner` from the decl's OWN `newtype_inner[decl]` entry — so
        // `Nominal{decl=B}.inner` is clean only if B's map entry is ALREADY normalized when A is rewritten.
        // In a chain A→B→C declared A-first, one originals-then-write-back pass bakes `Nominal{decl=B,
        // inner=<raw Sum{C}>}` into A, and the wasm backend recurses into `inner` (`valtype_of`/
        // `is_heap_type`) → the erased-newtype-read-as-boxed-handle miscompile (invalid wasm) at transitive
        // depth. Rewriting the whole map REPEATEDLY (each round reads the progressively-normalized map via
        // `normalize_sum`) drives every `inner` clean regardless of declaration order. TERMINATES: a round
        // only turns a raw keyed `Ty::Sum` into its `Nominal` (monotone, each occurrence flips at most once);
        // a recursive newtype's back-edge normalizes to the SAME one-level `Nominal{inner: template}` shape
        // every round (a fixed point, not re-expanded). Bounded by the key count as a non-convergence guard.
        let keyed: Vec<StructId> = db.newtype_inner.keys().copied().collect();
        for _ in 0..=keyed.len() {
            let mut changed = false;
            let mut renorm: Vec<(StructId, Ty)> = Vec::with_capacity(keyed.len());
            for &k in &keyed {
                let template = db.newtype_inner[&k].clone();
                let normalized = normalize_embedded_sums(&db, &template);
                if normalized != template {
                    changed = true;
                }
                renorm.push((k, normalized));
            }
            for (k, t) in renorm {
                db.newtype_inner.insert(k, t);
            }
            if !changed {
                break;
            }
        }
        // ENUM-DISCRIMINANT REPRESENTATION: materialize which sums are C-style enums — MORE than one
        // variant, EVERY variant nullary. Such a value is just its discriminant, so the backend
        // represents it as a bare `i32` (no heap box). A single-variant nullary sum is a nominal Unit
        // (already in `newtype_inner`), and a mixed sum stays boxed — so require ≥2 variants AND all
        // nullary here. Materialized after `newtype_inner` (an all-nullary decl with one variant went to
        // `newtype_inner` above and is not re-added), so the two sets are disjoint.
        for td in &db.type_decls {
            if td.variants.len() >= 2 && td.variants.iter().all(|v| v.payloads.is_empty()) {
                db.enum_disc.insert(td.occ);
            }
        }
        // EVICT the resolved/types cache residue from the `newtype_underlying` precompute above. That walk
        // called `typeval_of` on newtype payload occurrences (incl. a RECURSIVE newtype's self-reference)
        // BEFORE `newtype_inner` was fully populated, so it memoized `Resolved::TypeVal(Ty::Sum{decl})` on
        // the shared sum-record `(meta t)` nodes — the PRE-normalization boxed form. Left in place, a later
        // annotation `(: x Lst)` reads that stale `Ty::Sum` (bypassing `decode_ty`/`normalize_sum`) and
        // fails to unify with the value's `Ty::Nominal` ("type Lst does not match type Lst"). This
        // precompute is the ONLY thing that has touched these columns in `load`, so resetting them to
        // empty is safe — every real query re-decodes on demand through the now-complete `normalize_sum`,
        // getting the erased `Ty::Nominal`. (Cheap: the columns are otherwise empty at this point.)
        db.resolved = Column::new();
        db.types = Column::new();
        // The newtype precompute+fixpoint above is the only pre-normalization caller of `typeval_of`; it ran
        // with the memo OFF (a `Ty::Sum` reduced before `newtype_inner` was complete must not be cached).
        // Now `newtype_inner` is final and every reader agrees on `normalize_sum` — so from here on
        // `typeval_of` is deterministic per node and its result may be memoized into `db.typeval`.
        db.typeval_memo_live = true;
        db
    }

    /// Whether the sum whose declaration is `decl` is a C-style ENUM represented DIRECTLY as its
    /// discriminant `i32` (no heap box) — see [`Db::enum_disc`]. The single query every backend site uses
    /// to choose the unboxed path for construction, matching, equality, rendering, and boundary crossing.
    pub fn is_enum_disc(&self, decl: StructId) -> bool {
        self.enum_disc.contains(&decl)
    }

    /// Enter a β-reduction: on success returns a guard that holds the depth bumped for its lifetime;
    /// `None` if the depth is already at [`REDUCE_DEPTH_LIMIT`] — a reduction that deep is a diverging
    /// (recursive) fold, so the evaluator DECLINES. Dropping the guard leaves the reduction.
    pub fn enter_reduction(&mut self) -> Option<ReductionGuard<'_>> {
        if self.reduce_depth >= REDUCE_DEPTH_LIMIT {
            return None;
        }
        // TOTAL-WORK budget across the whole compile — the guard the per-reduction DEPTH counter cannot
        // give. A term can stay within the depth limit yet drive an EXPONENTIAL number of bounded
        // reductions: a self-applying lambda `(fn (v) (v (v v)))` applied to itself is not statically
        // recursive (it calls a PARAMETER, so `is_recursive` finds no cycle) and each reduction stays
        // shallow, but the type/fault walk over the shared, doubling term attempts ~2^depth reductions —
        // the compiler does unbounded work and appears to hang (the `cdz-smith` timeout). Every reduction
        // attempt funnels through here, so counting entries bounds that total work: past
        // [`REDUCE_NODE_BUDGET`] deny entry (like the depth limit), so the reduction DECLINES rather than
        // diverges. `reduce_nodes` is monotonic per compile (never decremented — it measures cumulative
        // work, not current depth); a real program's total reductions are far below the budget.
        if self.reduce_nodes >= REDUCE_NODE_BUDGET {
            return None;
        }
        self.reduce_nodes += 1;
        self.reduce_depth += 1;
        Some(ReductionGuard { db: self })
    }

    /// Like [`enter_reduction`] but for a TRIVIAL STRUCTURAL hop — a `Ref` dereference / a nominal-newtype
    /// unwrap: an O(1) step that follows one edge, NOT a β-reduction that can drive the exponential
    /// bounded-reduction fanout the cumulative [`REDUCE_NODE_BUDGET`] exists to bound (a self-applying
    /// lambda `(fn (v) (v (v v)))`). So this charges ONLY the per-chain DEPTH guard (`REDUCE_DEPTH_LIMIT`,
    /// which terminates a `Ref`/value cycle — `(def (g) g)`), NOT the whole-compile node budget.
    ///
    /// WHY (the bug this fixes): `reduce_nodes` is CUMULATIVE over a whole `compile()` and never reset, so a
    /// large-but-TERMINATING multi-module closure build (the compiler-ml self-host: emit-db + every imported
    /// module monomorphized in one compile) legitimately exceeds the 1M budget. Once exhausted,
    /// `enter_reduction` denied EVERY subsequent reduction — including a trivial `Ref{Option}→record` hop in
    /// `member_value(Option, "None")` — so `reduce_to_record_id` returned `None`, `Option.None` mis-typed as
    /// "a Option value has no field `None`", and a WELL-TYPED program failed its downstream component build
    /// with locationless CDZ0201 (layout-sensitive: the fault fired once the budget happened to run out at
    /// that call). Charging a structural hop against an EXPONENTIAL-fanout budget was the category error; a
    /// `Ref` hop does constant work and only needs cycle termination. The `Apply` β-reduction arms keep the
    /// full [`enter_reduction`] (the budget still bounds the real divergence — the `cdz-smith` timeout).
    pub fn enter_reduction_structural(&mut self) -> Option<ReductionGuard<'_>> {
        if self.reduce_depth >= REDUCE_DEPTH_LIMIT {
            return None;
        }
        // PER-DEF-BODY structural-reduction WORK bound: the structural path is not charged against
        // `REDUCE_NODE_BUDGET`, so a self-app whose type/fault walk drives an exponentially-widening
        // structural re-reduction escapes that guard and HANGS. Count structural reductions (reset per
        // def body in `collect_faults`); past the ceiling, deny entry — the walk declines with a
        // resource-limit CDZ0999 instead of diverging. The count grows WITH the exploding width → trips
        // BEFORE the width materializes (fast), while a real body stays far below the ceiling.
        if self.structural_reductions >= STRUCTURAL_REDUCTION_BUDGET {
            return None;
        }
        self.structural_reductions += 1;
        self.reduce_depth += 1;
        Some(ReductionGuard { db: self })
    }

    /// Push a flow-sensitive refinement FRAME for a control-flow branch being emitted (see
    /// [`range_refinements`]). The caller MUST call [`pop_range_refinements`] on every exit path — the
    /// emit `if` arm brackets a branch's emission with a push/emit/pop so an early `?` return still pops.
    /// A frame is the FULL in-scope set (the caller merges the parent frame in), so lookups read only the
    /// top. An empty `frame` is still pushed (so the pop count matches) — it simply refines nothing.
    ///
    /// [`range_refinements`]: Db::range_refinements
    /// [`pop_range_refinements`]: Db::pop_range_refinements
    pub(crate) fn push_range_refinements(
        &mut self,
        frame: crate::fxhash::FxHashMap<StructId, ValueFact>,
    ) {
        self.range_refinements.push(frame);
    }

    /// Pop the current refinement frame (pairs with [`push_range_refinements`]).
    ///
    /// [`push_range_refinements`]: Db::push_range_refinements
    pub(crate) fn pop_range_refinements(&mut self) {
        self.range_refinements.pop();
    }

    /// The flow-sensitive refined range for the binder `b` in the CURRENT branch, if any — the top
    /// refinement frame's entry. `None` when no branch context is active (the const-fold callers) or the
    /// binder is not refined here, so the caller falls back to the declared-type / arith range.
    pub(crate) fn refined_range(&self, b: StructId) -> Option<(i64, Option<i64>)> {
        // Project out the `int_range` facet — slice 1 populates only that, so this is the same interval
        // the interval consumers saw before the `ValueFact` widening.
        self.range_refinements.last()?.get(&b)?.int_range
    }

    /// The COLLECTION binder that the index binder `b` is flow-known to be `< len(...)` of, in the CURRENT
    /// branch (the below-len facet). `Some(coll)` ⇒ an enclosing `(< b (List.len coll))` guard proved
    /// `b < len(coll)` here, so a `List.at coll b` can shed its own `index < len` check. `None` when no
    /// branch context is active or `b` carries no below-len fact — the caller keeps the runtime check.
    /// Reads only the TOP frame (like `refined_range`): the fact is branch-scoped, pushed on entry and
    /// popped on exit, never merged across a control-flow join.
    pub(crate) fn refined_below_len(&self, b: StructId) -> Option<StructId> {
        self.range_refinements.last()?.get(&b)?.below_len
    }

    /// The current (top) refinement frame, or an empty map — so the `if` emit can clone it as the base a
    /// child branch extends (each branch's frame = parent's refinements ∪ this branch's).
    pub(crate) fn current_refinements(&self) -> crate::fxhash::FxHashMap<StructId, ValueFact> {
        self.range_refinements.last().cloned().unwrap_or_default()
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

    /// The file index a node resolves its CONSTRUCTOR/TYPE VISIBILITY against — `file_of(at)`, but when
    /// `at` itself is in NO file (an appended synthesized node — an eval/quote-reconstructed form, a
    /// tagged-template expansion), walk up `parent_of` to the nearest ANCESTOR that IS in a file. The
    /// expansion desugars (`eval_ast::desugar_eval`, `tagged_template::expand`) splice their reconstructed
    /// source under the ORIGINAL call node (`(eval …)`/`(tagged-template …)`), whose `StructId` is in its
    /// own file's demux range, so the parent chain of a reconstructed constructor reference leads back to
    /// the CONSUMER file. Resolving its visibility there is the metaprogramming guarantee that expanded AST
    /// is checked "exactly as if it had been written directly" (`metaprogramming.md` §Expansion Precedes
    /// And Feeds The Core Guarantees) — so a module-PRIVATE constructor named through `(eval (quote …))`
    /// from outside its module is withheld (CDZ0214) exactly as a hand-written `(. T Ctor)` is, closing the
    /// abstract-type forge hole (eval must get NO privileged scope). `None` only if no ancestor has a file
    /// (a wholly prelude/link-root subtree — indeterminate, caller falls back to the flat table).
    pub(crate) fn visibility_file_of(&self, at: StructId) -> Option<usize> {
        let fs = self.file_scope.as_ref()?;
        let mut cur = Some(at);
        while let Some(id) = cur {
            if let Some(file) = fs.file_of(id) {
                return Some(file);
            }
            cur = self.parent_of(id);
        }
        None
    }

    /// Graft `node`'s root under `new_parent` (at child position `child_ix`) so a scope walk from a name
    /// INSIDE `node` continues up into `new_parent`'s enclosing scope. Used when a fold synthesizes a
    /// replacement subtree (the effects `reduce_handle` body) that must resolve its FREE variables against
    /// the site the original form occupied — e.g. a `handle` body referencing an enclosing function's
    /// parameter. `push_list` leaves a synthesized root's parent `None` (self-contained), which strands a
    /// free reference as CDZ0101; re-anchoring it to the original site's parent restores the lexical chain.
    /// Only the ROOT is re-parented — its children already point at it. A no-op if `node` is out of range.
    pub fn reparent(&mut self, node: StructId, new_parent: Option<StructId>, child_ix: u32) {
        let ix = node.0 as usize;
        if ix < self.parent.len() {
            self.parent[ix] = new_parent;
            self.child_ix[ix] = child_ix;
        }
    }

    /// Give a SYNTHESIZED subtree scope-skip entries so a name resolved inside it hops O(1) to the nearest
    /// real binding candidate instead of walking every enclosing synth form. `push_list` deliberately
    /// leaves synth nodes UNCOVERED (the exhaustive parent walk is cheap for a SHALLOW β-reduced copy — its
    /// usual case), but a desugar that builds a DEEP self-contained chain (the runtime map-match's N-nested
    /// `(if <k-present> body <else>)` presence chain) breaks that: each synth node is a NON-binding
    /// `if`/`match`/`.`/application form, so a prelude name (`Map`/`Some`/`None`/…) in the innermost arm
    /// walks O(depth) forms to conclude "not lexically bound", and with N arms that is O(N²).
    ///
    /// Every node in such a chain is a non-binding form (its only real binders are the REUSED arm bodies,
    /// which resolved — and memoized — BEFORE the desugar, so they never re-resolve here). So each fresh
    /// node's skip is simply its PARENT's skip (path-compression over the non-binding spine, exactly what
    /// `build_scope_skip` computes for a non-candidate) — bottoming at the chain root's skip. The root is
    /// self-contained (parent `None` after `push_list`), so its names fall through to the prelude, which is
    /// where `Map`/`Some`/`None`/`match`/`if` resolve anyway. Idempotent + bounded to `root`'s subtree.
    pub fn extend_scope_skip_pass_through(&mut self, root: StructId) {
        // Grow the skip vector to cover every id up to the current arena length (synth nodes appended since
        // load), defaulting to `None` (no candidate above → prelude fallback), then fill `root`'s subtree
        // top-down: a child inherits its parent's (now-final) skip entry, since a synth form binds nothing.
        // A node BELOW `covered_boundary` was covered by the load-time `build_scope_skip` (or a prior
        // extension) — it keeps its own final entry; only ids at/above the boundary are fresh synth nodes.
        let covered_boundary = self.scope_skip.len();
        let len = self.ast.structure.len();
        if self.scope_skip.len() < len {
            self.scope_skip.resize(len, None);
        }
        // A root already covered by the load-time index (or a prior extension) keeps its own entry — only
        // a fresh synth root needs seeding. Nothing to do if it is out of range.
        if (root.0 as usize) < covered_boundary || (root.0 as usize) >= self.scope_skip.len() {
            return;
        }
        // The root's own skip: whatever its parent sees (usually `None` — a freshly-pushed root is
        // parentless), so a name in the chain resolves against the root's lexical context (the prelude).
        let root_skip = match self.parent_of(root) {
            Some(p) if (p.0 as usize) < self.scope_skip.len() => self.scope_skip[p.0 as usize],
            _ => None,
        };
        self.scope_skip[root.0 as usize] = root_skip;
        // Mark the root as GENUINELY seeded (its entry is computed, not a resize-default), so the fast-path
        // is safe for it. Without this, a synth node's entry is indistinguishable from the `None` an
        // unrelated resize leaves behind. See `scope_skip_covers`.
        self.scope_skip_seeded.insert(root);
        // Descend: each node passes its skip down to its children (a non-binding synth form is transparent
        // to scope). A child that is a LOAD-TIME node (a reused body occurrence) already has its own final
        // entry — do not overwrite it; only fill fresh (previously-`None`, past-load) synth nodes.
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let node_skip = self.scope_skip[node.0 as usize];
            if let Struct::List(children) = self.ast.get(node) {
                for &c in children.clone().iter() {
                    let ci = c.0 as usize;
                    // Only propagate INTO fresh synth nodes (a reused load-time / already-covered child
                    // keeps its own final skip — do not overwrite it).
                    if ci >= covered_boundary && ci < self.scope_skip.len() {
                        self.scope_skip[ci] = node_skip;
                        self.scope_skip_seeded.insert(c);
                        stack.push(c);
                    }
                }
            }
        }
    }

    /// CANDIDATE-AWARE scope-skip extension for a synthesized subtree — the generalization of
    /// [`extend_scope_skip_pass_through`] for a chain that CONTAINS a binding form. Where the pass-through
    /// version assumes EVERY node is non-binding (so every child inherits its parent's skip), this applies
    /// [`build_scope_skip`]'s exact per-node rule: a child of a BINDING-CANDIDATE parent skips TO that
    /// parent (`(parent, child)` — so a reference inside it lands on the binder), otherwise it inherits the
    /// parent's skip (path-compression over the non-binding spine). The refutable-literal-element desugar
    /// (`lower::desugar_refutable_literal_list_elements`) builds a `(guard (list …) (and (and (= __le0 …)
    /// …) …))` arm whose guard COND is an O(N)-DEEP left-nested `and`-chain; without coverage each `__leK`
    /// guard reference (and each prelude `=`/`and`) walks O(depth) `and` forms to the enclosing `(guard …)`
    /// → O(N²). This lands every inner reference on the nearest real candidate (the `guard`, which binds the
    /// element binders via `binder_in`'s Case 6lg) in O(1). Idempotent + bounded to `root`'s subtree.
    ///
    /// SOUND for BOTH the map-match chain (all non-binding → identical to the pass-through version) and a
    /// guard chain (the one `guard` node becomes a candidate its children skip TO). The pass-through version
    /// is retained for its caller; a future consolidation could route both here.
    pub fn extend_scope_skip_into_subtree(&mut self, root: StructId) {
        let covered_boundary = self.scope_skip.len();
        let len = self.ast.structure.len();
        if self.scope_skip.len() < len {
            self.scope_skip.resize(len, None);
        }
        if (root.0 as usize) < covered_boundary || (root.0 as usize) >= self.scope_skip.len() {
            return;
        }
        // The root's own skip: the SAME per-node rule the descent below applies to children, but to
        // root's PARENT — if the parent is a binding CANDIDATE, root skips TO it (`(parent, root)`, so a
        // name unbound within the subtree is looked up in the parent's bindings), else root inherits the
        // parent's own skip (path-compression over a non-binding parent). Using the parent's skip
        // UNCONDITIONALLY skipped PAST a binding parent: a desugar's rewritten `(match …)` reparented into
        // an enclosing `let`'s body slot then never saw that `let`'s bindings, so a guard-cond reference to
        // it exhausted the skip → spurious CDZ0101 (the inlined guarded-literal-list bug). A parentless root
        // (the usual freshly-pushed case) still gets `None` → falls through to its lexical context.
        let root_skip = match self.parent_of(root) {
            Some(p) if (p.0 as usize) < self.scope_skip.len() => {
                if is_binding_candidate(&self.ast, &self.parent, &self.module_records, p) {
                    Some((p, root))
                } else {
                    self.scope_skip[p.0 as usize]
                }
            }
            _ => None,
        };
        self.scope_skip[root.0 as usize] = root_skip;
        // Mark the root as GENUINELY seeded (computed entry, not a resize-default) — the fast-path is safe
        // for it. See `scope_skip_covers`.
        self.scope_skip_seeded.insert(root);
        // Descend top-down: for each fresh child, its skip = `(node, child)` if `node` is a binding
        // candidate (land inner references on `node`), else `node`'s own skip (hop over the non-binding
        // spine). This is `build_scope_skip`'s unwind rule, applied to the synth subtree.
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let node_skip = self.scope_skip[node.0 as usize];
            let node_is_candidate =
                is_binding_candidate(&self.ast, &self.parent, &self.module_records, node);
            if let Struct::List(children) = self.ast.get(node) {
                for &c in children.clone().iter() {
                    let ci = c.0 as usize;
                    // Only fill fresh synth children (a reused load-time / already-covered child keeps its
                    // own final skip — never overwrite it).
                    if ci >= covered_boundary && ci < self.scope_skip.len() {
                        self.scope_skip[ci] = if node_is_candidate {
                            Some((node, c))
                        } else {
                            node_skip
                        };
                        self.scope_skip_seeded.insert(c);
                        stack.push(c);
                    }
                }
            }
        }
    }

    /// The source NAME of a `let` binding whose value is the occurrence `init`. A `let` binding is a
    /// two-element pair `(name init)` in the bindings list, and a kept `Core::Let` binding is keyed by
    /// its `init` occurrence (see `lower_let`) — so the name is `init`'s FIRST sibling. Reached in O(1)
    /// via `parent_of` (the enclosing pair) + its first child; used by the DWARF backend to give a kept
    /// scalar `let` local a `DW_TAG_variable` DIE. Returns `None` when `init` is not a pair's value.
    pub fn let_binding_name(&self, init: StructId) -> Option<&str> {
        let pair = self.parent_of(init)?;
        let Struct::List(kv) = self.ast.get(pair) else {
            return None;
        };
        // Must be a genuine `(name init)` pair with `init` in the value slot — not, say, a call whose
        // head happens to be a name and `init` an argument.
        if kv.len() == 2 && kv[1] == init {
            self.ast.as_name(kv[0])
        } else {
            None
        }
    }

    /// The BINDER NAME a scalar match arm binds, given the arm's BODY occurrence. A match arm is a
    /// two-element `(pattern body)` list; a binder pattern is either a bare name `x` or a guard
    /// `(guard x cond)` whose first child is the binder (`resolve` Case 5 / 5g). The binder binds the
    /// whole scrutinee, so the DWARF backend describes it as a `DW_TAG_variable` at the scrutinee's slot,
    /// scoped to the match's lexical block. Returns `None` when the pattern is a literal, the wildcard
    /// `_`, or `body` is not an arm body — the cases with no name to inspect. Mirrors `let_binding_name`
    /// (reached O(1) via `parent_of` + a child read).
    pub fn match_arm_binder_name(&self, body: StructId) -> Option<&str> {
        let arm = self.parent_of(body)?;
        let Struct::List(pb) = self.ast.get(arm) else {
            return None;
        };
        // A genuine `(pattern body)` arm with `body` in the body slot (not the pattern position).
        if pb.len() != 2 || pb[1] != body {
            return None;
        }
        // The binder pattern: a bare name, or the first child of a `(guard binder cond)`.
        let binder_pat = match self.ast.as_form(pb[0], "guard") {
            Some(g) if g.len() == 2 => g[0],
            _ => pb[0],
        };
        match self.ast.as_name(binder_pat) {
            // The wildcard `_` binds nothing to inspect; a literal pattern is not a name at all.
            Some("_") | None => None,
            Some(name) => Some(name),
        }
    }

    /// Whether `id` is `ancestor` or lexically WITHIN its subtree — walk `id`'s parent chain and see if
    /// `ancestor` is on it. Used by the closure-capture check to tell a reference to the lambda's OWN
    /// scope (its param, a nested `let`) from a reference OUTSIDE it (a captured free variable). O(depth)
    /// via the same parent chain the scope walk uses; a synthesized node past the parent index stops the
    /// walk (returns false — treated as not-within, so a capturing lambda declines conservatively).
    pub fn is_within(&self, id: StructId, ancestor: StructId) -> bool {
        let mut cur = id;
        loop {
            if cur == ancestor {
                return true;
            }
            match self.parent_of(cur) {
                Some(p) => cur = p,
                None => return false,
            }
        }
    }

    /// Register a LAMBDA-LIFTED closure, returning its funcref-TABLE slot (its position in `db.lifted`).
    /// Deduped by the lambda's `body` occurrence — a lambda lifted more than once (a combinator used at
    /// several sites, or re-lowered) keeps ONE table slot + ONE emitted function. See [`lifted`].
    ///
    /// [`lifted`]: Db::lifted
    pub fn lift_lambda(&mut self, lam: crate::lower::LiftedLambda) -> usize {
        // Dedup by the lambda's `body` occurrence via the O(1) index (was a linear scan of `lifted` →
        // O(N²) for N distinct lifted closures). The index and `lifted` stay in lockstep: a body already
        // present returns its slot; a new one is pushed and indexed at its position.
        if let Some(&pos) = self.lifted_by_body.get(&lam.body) {
            return pos;
        }
        let pos = self.lifted.len();
        self.lifted_by_body.insert(lam.body, pos);
        self.lifted.push(lam);
        pos
    }

    /// Whether the lexical-scope SKIP index covers `id` — i.e. whether `id`'s [`scope_skip`] entry is
    /// GENUINE (so the fast-path may be taken) rather than a `resize`-default. True for a LOAD-TIME node
    /// (below the load boundary — `build_scope_skip` computed its entry) and for a post-load SYNTH node
    /// that was EXPLICITLY seeded ([`scope_skip_seeded`]). FALSE for an unseeded synth node — even one the
    /// vector was grown past by an unrelated `extend_scope_skip_*` resize: its `None` entry there is a
    /// default, not a computed "resolves to prelude", so it must fall back to the exhaustive parent walk.
    /// Without this distinction a synth node (a β-copied spec body, a lifted closure body) swept into the
    /// vector by a later seeding would take the fast-path, bottom at its default `None`, and spuriously
    /// report its own enclosing binder UNBOUND (a CDZ0101). See [`scope_skip`].
    ///
    /// [`scope_skip`]: Db::scope_skip
    /// [`scope_skip_seeded`]: Db::scope_skip_seeded
    pub fn scope_skip_covers(&self, id: StructId) -> bool {
        // A node explicitly FORCED uncovered (a load-time occurrence re-parented into a synth scope, whose
        // load-time skip entry is now stale) must take the exhaustive walk. Gated on non-empty so the common
        // case (no forced nodes) pays only an `is_empty` check, not a hash lookup, on this hot path.
        if !self.scope_skip_force_uncovered.is_empty()
            && self.scope_skip_force_uncovered.contains(&id)
        {
            return false;
        }
        ((id.0 as usize) < self.scope_skip_load_boundary) || self.scope_skip_seeded.contains(&id)
    }

    /// Force every LOAD-TIME node in `root`'s subtree to fall back to the exhaustive lexical-scope walk
    /// (see [`scope_skip_force_uncovered`]). Call after RE-PARENTING a subtree that contains load-time name
    /// occurrences into a freshly-synthesized scope (the escaped-closure recovery's `let` rebuild), so those
    /// references resolve against the CURRENT parent chain instead of their stale load-time skip entry. Only
    /// load-time ids are marked — a synth node's coverage already derives from `scope_skip_seeded`, which the
    /// rebuild set correctly. Safe (the walk is exhaustive-correct) and idempotent.
    pub fn force_structural_resolution_subtree(&mut self, root: StructId) {
        let boundary = self.scope_skip_load_boundary;
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if (node.0 as usize) < boundary {
                self.scope_skip_force_uncovered.insert(node);
            }
            if let Struct::List(children) = self.ast.get(node) {
                for &c in children.clone().iter() {
                    stack.push(c);
                }
            }
        }
    }

    /// True iff `form` is an INDEXED match arm that PROVABLY cannot bind `name` — i.e. `name` does not
    /// occur as a NAME atom anywhere in the arm's pattern region (all children except the body). Lets
    /// `binder_in` fast-reject a scope-walk hop over such an arm in O(1), skipping the ~20-case arm
    /// cascade. Returns `false` when `form` is not an indexed arm (e.g. a synthesized β-copy arm absent
    /// from the load-time index, or any non-arm candidate) — then the caller runs the full cascade, which
    /// is always correct. SAFE by construction: the index over-approximates the arm's bound names, so a
    /// name the arm can genuinely bind is never reported as "cannot bind". See [`arm_pattern_names`].
    ///
    /// [`arm_pattern_names`]: Db::arm_pattern_names
    pub fn arm_cannot_bind(&self, form: StructId, name: &str) -> bool {
        match self.arm_pattern_names.get(&form) {
            Some(names) => !names.contains(name),
            None => false,
        }
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

    /// The value occurrence of the LAST bare binding of `name` in the let bindings-list `bindings_occ`
    /// that lies STRICTLY BEFORE child position `end` — the O(log N) index answer to `last_binder_named`'s
    /// reverse scan, for a bindings-list whose bindings are all bare/annotated-bare names (the only lists
    /// present in [`let_binder_index`]). `Some(None)` means the list IS indexed but has no such binding
    /// before `end` (a definitive negative — do NOT fall back to the linear scan). `None` means the list
    /// is NOT indexed (it has a destructuring binding) — the caller must run the linear walk.
    ///
    /// `end` is the exclusive window end (the count of in-scope pairs), matching `last_binder_named`'s
    /// `end`. The positions are stored ASCENDING, so `partition_point(pos < end)` finds the count of
    /// candidates in the window and the last of them (the highest position < `end`) is the last-wins
    /// binder — byte-identical to the reverse scan's "first match walking backward from `end`".
    pub fn let_binder_before(
        &self,
        bindings_occ: StructId,
        name: &str,
        end: usize,
    ) -> Option<Option<StructId>> {
        let by_name = self.let_binder_index.get(&bindings_occ)?;
        let Some(positions) = by_name.get(name) else {
            // The list is indexed but never binds `name` — a definitive negative (O(1)).
            return Some(None);
        };
        let k = positions.partition_point(|&(pos, _)| (pos as usize) < end);
        Some(k.checked_sub(1).map(|last| positions[last].1))
    }

    /// The do-local `def`/`module` forms declaring `name` in the `(do …)`/`(module …)` at `scope_form`,
    /// as ascending `(position, form)` pairs. Replaces `binder_in`'s / `module_sibling_binds`' O(forms)
    /// scans (see [`do_binder_index`]) with a lookup over ONLY the (usually one) declaring forms.
    ///
    /// Returns `None` when the scope is NOT INDEXED — a scope form appended AFTER load (a copied do-block
    /// from inlining a recursive do-local function's body, whose occ is past the load-time arena), which
    /// the load-time index cannot contain. The caller must then fall back to the live per-form scan. This
    /// is DISTINCT from an indexed scope that simply does not declare `name`, which returns `Some(&[])` (a
    /// definitive O(1) negative — no fallback). So: `None` ⇒ fall back; `Some([])` ⇒ absent; `Some(xs)` ⇒
    /// the declaring forms.
    pub fn do_forms_declaring(
        &self,
        scope_form: StructId,
        name: &str,
    ) -> Option<&[(u32, StructId)]> {
        let by_name = self.do_binder_index.get(&scope_form)?;
        Some(by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[]))
    }

    /// EVERY parameter binder a scope form declares — `(name, name-occurrence)` pairs for a `fn`/`def`'s
    /// parameters. The enumeration companion of [`binder_in_scope`] (which looks one up): a scope query
    /// (`ScopeAt`) needs ALL names visible at a point, not one. Empty iterator for a non-scope form.
    pub fn scope_binders_of(&self, scope: StructId) -> impl Iterator<Item = (&str, StructId)> {
        self.scope_binders
            .get(&scope)
            .into_iter()
            .flat_map(|m| m.iter().map(|(n, &occ)| (n.as_str(), occ)))
    }

    /// Whether `id` is a genuine USER-PROGRAM node — one the decoded program contained, which the
    /// front-end's span table can map to a text region. A prelude binding or an evaluator-synthesized
    /// node (β-reduced body, built `(Int W)` module) is NOT one: it has no source position, so its id
    /// must never reach a diagnostic the consumer maps. The diagnostic edge filters an origin through
    /// this so a synthesized/prelude id is dropped (reported as unanchored) rather than mis-mapped.
    pub fn is_user_node(&self, id: StructId) -> bool {
        id.0 < self.user_node_count
    }

    /// The number of USER-PROGRAM nodes — the `[0, user_node_count)` `StructId` range the decoded program
    /// contained, below the appended prelude + evaluator-synthesized nodes. A whole-program check that can
    /// only report a fault at a user node (its origin needs a source span) scans this range, not the full
    /// `ast.structure.len()`, so it skips the built-in bulk. See [`Db::is_user_node`].
    pub(crate) fn user_node_count(&self) -> u32 {
        self.user_node_count
    }

    /// The USER (source) node a synthesized node ultimately traces to, or `None` if there is no recorded
    /// provenance ending at a user node. A β-reduction NAME COPY records its source occurrence in
    /// `synth_name_origin` (`copy_structural`); this follows that chain (a copy-of-a-copy re-points to the
    /// original) until it reaches a `is_user_node`, so a diagnostic anchored at an inlined copy can be
    /// RELOCATED to the real source reference the author wrote instead of being dropped as unmappable. A
    /// bounded walk (the chain is at most the reduction depth; the loop guard stops a cycle defensively).
    /// Returns `None` for an already-user node (nothing to relocate) or a synth node with no provenance.
    pub fn source_of_synth(&self, id: StructId) -> Option<StructId> {
        if self.is_user_node(id) {
            return None;
        }
        let mut cur = id;
        // A β-copy chain is at most reduction-depth long; cap the follow at the arena size as a cycle
        // backstop (a self-referential provenance entry could otherwise spin — it never should, but the
        // guard keeps this total on any map state).
        for _ in 0..self.ast.structure.len() {
            match self.synth_name_origin.get(&cur) {
                Some(&src) if self.is_user_node(src) => return Some(src),
                Some(&src) if src != cur => cur = src,
                _ => return None,
            }
        }
        None
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
        self.push_atom(Leaf::Name(name.into()))
    }

    /// Append an `Atom` of a STRING literal. A synthesized primitive-constructor head (`"tuple"`/
    /// `"record"`) is a string, not a name — unspellable as an identifier, so unshadowable.
    pub fn push_str(&mut self, s: &str) -> StructId {
        self.push_atom(Leaf::Str(s.into()))
    }

    /// Append a canonical `(= key value)` FIELD PAIR node — the shape shared by record fields and map
    /// entries. The runtime-construction twin of [`crate::ast::Arenas::field_pair`] /
    /// [`crate::ast::Builder::field_pair`], so synthesized record/map forms route through one place.
    /// Emits the NATIVE `FieldPair` leaf head (recognized by kind), matching what the reader now produces;
    /// the `as_name` migration bridge still spells it `"="` for any legacy name-head consumer.
    pub fn push_field_pair(&mut self, key: StructId, value: StructId) -> StructId {
        let eq = self.push_atom(Leaf::FieldPair);
        self.push_list(vec![eq, key, value])
    }

    /// Append a native compound-constructor form `(#<ctor> child…)` — the runtime-construction twin of
    /// [`crate::ast::Builder::compound`], emitting the payloadless ctor-LEAF head (recognized by kind, not
    /// head text). Synthesized record/list/tuple/map/set values route through here so they carry the M2
    /// native head rather than a legacy `"record"` string / `record` name alias.
    pub fn push_compound(&mut self, ctor: CompoundCtor, children: Vec<StructId>) -> StructId {
        let head = self.push_atom(Leaf::Ctor(ctor));
        let mut nodes = Vec::with_capacity(1 + children.len());
        nodes.push(head);
        nodes.extend(children);
        self.push_list(nodes)
    }

    /// The definition of the given name, if one exists — how an export resolves its target and how a
    /// later call resolves its callee (by name against the index, reading a signature, not a body).
    /// O(1) via the `def_name_index` map (built at load). `resolve_name` calls this for every non-local
    /// name reference, so a linear scan here made resolution O(N²) on a many-def program.
    pub fn def_by_name(&self, name: &str) -> Option<usize> {
        self.def_name_index.get(name).copied()
    }

    /// The GLOBAL (`Project.cdz` manifest) overflow default for an operand of the given signedness, or `None`
    /// if the manifest sets none (→ the built-in `Trap`). The precedence level between a module
    /// `(pragma overflow …)` and `Trap`; read by `infer::overflow_mode_of` when a node has no governing
    /// module spec. See [`Db::global_overflow`].
    pub fn global_overflow_default(&self, unsigned: bool) -> Option<OverflowMode> {
        if unsigned {
            self.global_overflow.unsigned
        } else {
            self.global_overflow.signed
        }
    }

    /// The `(doc "…")` text captured off the definition whose SIGNATURE occurrence is `sig_occ`, or
    /// `None` if that definition carried no doc. The read behind the `DocOf`/`DocAt` queries: a definition
    /// keeps its documentation reachable even though `strip_def_docs` removes the doc form from its body.
    /// Key by `Def::sig_occ` — the same node a name lookup and a body lookup both resolve to.
    ///
    /// This is how the compiler EXPOSES a definition's documentation in a machine-readable form — the
    /// `cdz doc`/`doc-at` queries read this column, returning the text as data rather than re-scanning
    /// source, so an agent gets a definition's documentation as a first-class query result.
    //= spec/capabilities/agent-authoring.md#documentation-is-machine-readable
    //# The compiler MUST expose the documentation attached to a definition in a machine-readable form.
    pub fn doc_of_def(&self, sig_occ: StructId) -> Option<&str> {
        self.doc_by_sig.get(&sig_occ).map(String::as_str)
    }

    /// Push a SYNTHESIZED def (an effect specialization `f#ctx`, `crate::effects` E3) and index it by
    /// name, returning its index. Its body may be `None` at push time (reserved, filled by
    /// [`fill_specialized_def`] once threaded) so a recursive self-call inside the body can name it. The
    /// name is unspellable in source (`#` in it), so it never collides with a user def.
    pub(crate) fn push_specialized_def(&mut self, def: Def) -> usize {
        let idx = self.defs.len();
        self.def_name_index.entry(def.name.clone()).or_insert(idx);
        if let Some(body) = def.body {
            self.def_by_body.insert(body, idx);
        }
        self.defs.push(def);
        idx
    }

    /// Fill a reserved specialized def's parameters + body (see [`push_specialized_def`]) — the two-step
    /// build a self-referential specialization needs: reserve the name, thread the body (which may call
    /// the name), then fill. Updates the body index so `def_index_by_body` finds it.
    pub(crate) fn fill_specialized_def(
        &mut self,
        idx: usize,
        params: Vec<StructId>,
        body: StructId,
    ) {
        self.defs[idx].params = params;
        self.defs[idx].body = Some(body);
        self.def_by_body.insert(body, idx);
    }

    /// File-scoped def lookup for a multi-file PACKAGE (`DESIGN-package-linking.md` §4). Resolves the
    /// VALUE name `name`, referenced from node `at`, to a def index — but ONLY the defs visible in
    /// `at`'s own file (its own top-level defs + its imports), never a sibling file's defs. Returns:
    ///  - `Some(Ok(idx))` — `name` resolves to def `idx` in `at`'s file scope;
    ///  - `Some(Err(()))` — a package is linked and `at`'s file is KNOWN, but `name` is not visible there (caller must NOT fall back to the global scan — that would leak a sibling's def; fall through to the prelude / unbound instead);
    ///  - `None` — no package linkage, OR `at` is a synthesized / β-copied / prelude node whose file is indeterminate; caller uses the flat `def_by_name` path.
    ///
    /// The `None`-for-indeterminate case is the β-copy-hygiene fallback: a copied node lies outside
    /// every `FileSpan`, so its file is unknown here; the resolver handles that separately (a name
    /// defined in exactly one file across the whole package is unambiguous and resolves flat; a name
    /// defined in more than one file is ambiguous under a copy and DECLINES rather than guessing).
    pub(crate) fn file_scoped_def(&self, at: StructId, name: &str) -> Option<Result<usize, ()>> {
        let fs = self.file_scope.as_ref()?;
        let file = fs.file_of(at)?;
        Some(fs.visible[file].get(name).copied().ok_or(()))
    }

    /// The SPLICED module index a WHOLE-MODULE ALIAS `(import "path" alias)` binds `alias` to, as seen from
    /// node `at`'s file — or `None` if `alias` is not a module alias here (a value/type name, or no
    /// linkage). Used by `resolve_member` to project `(. alias member)` against the aliased module's
    /// exports. File-scoped via `visibility_file_of` (an alias is visible only in the file that imported it).
    pub(crate) fn module_alias_target(&self, at: StructId, alias: &str) -> Option<usize> {
        let fs = self.file_scope.as_ref()?;
        let file = self.visibility_file_of(at)?;
        fs.module_aliases.get(file)?.get(alias).copied()
    }

    /// The def INDEX a `from_file` module exports under `key` (its own defs + re-exported imports) — the
    /// exact `visible` surface a named import binds from. `None` if `from_file` does not export a VALUE def
    /// `key`. The projection target for a module-alias `(. alias key)`.
    pub(crate) fn export_def_in_file(&self, from_file: usize, key: &str) -> Option<usize> {
        let fs = self.file_scope.as_ref()?;
        fs.visible.get(from_file)?.get(key).copied()
    }

    /// File-scoped TYPE-name lookup for a multi-file package — the type analogue of `file_scoped_def`.
    /// Resolves the type NAME `name`, referenced from node `at`, to the synth-record occurrence it names,
    /// but ONLY the types visible in `at`'s own file (its own `(type …)` declarations + the types it
    /// imports), never a sibling file's type. Returns:
    ///  - `Some(Ok(occ))` — `name` resolves to the type whose synth record is `occ` in `at`'s file scope;
    ///  - `Some(Err(()))` — a package is linked and `at`'s file is KNOWN, but `name` is not a visible type there (fall through; do NOT leak a sibling's type — this is the privacy);
    ///  - `None` — no linkage, OR `at` is synthesized / β-copied / prelude (indeterminate file); caller uses the flat `type_decl_by_name`.
    pub(crate) fn file_scoped_type(
        &self,
        at: StructId,
        name: &str,
    ) -> Option<Result<StructId, ()>> {
        let fs = self.file_scope.as_ref()?;
        let file = self.visibility_file_of(at)?;
        Some(fs.visible_types[file].get(name).copied().ok_or(()))
    }

    /// File-scoped bare-VARIANT-CONSTRUCTOR lookup for a multi-file package — the ctor analogue of
    /// `file_scoped_def`. Resolves a bare variant name (`Red` for `(type Color (Red) …)`) to its ctor
    /// node, but ONLY the constructors visible in `at`'s own file (own type decls + imported types).
    /// Same tri-state as `file_scoped_type`; `Some(Err(()))` means the file is known and the ctor is not
    /// visible there (fall through — a same-named prelude ctor like `Some` may still apply).
    pub(crate) fn file_scoped_variant_ctor(
        &self,
        at: StructId,
        name: &str,
    ) -> Option<Result<StructId, ()>> {
        let fs = self.file_scope.as_ref()?;
        let file = self.visibility_file_of(at)?;
        Some(fs.visible_ctors[file].get(name).copied().ok_or(()))
    }

    /// Whether the variant ctor `ctor_name` is WITHHELD at `at`'s file — the type's HANDLE is visible here
    /// but this constructor was NOT exported (an abstract / partially-concrete import). True ⇒ a bare
    /// construct `(A v)` or bare match pattern `((A v))` through `A` must be CDZ0214, exactly as the
    /// qualified `(. T A)` selector is (`resolve::withheld_ctor_reject`) — else one spelling reads private
    /// ADT internals (encapsulation SOUNDNESS; the verification kernel's Thm/Term trust). False when
    /// `ctor_name` IS a visible ctor here (legitimate) — checked against BOTH the bare and qualified surfaces
    /// (the qualified retains a prelude-named ctor the bare map omits, so `Ast.Int` is not mis-flagged).
    /// Only meaningful for a linked package. `owner_type` (a file-visible type declaring `ctor_name`) is the
    /// name to blame in the message; caller already knows the scrutinee type so it usually passes that.
    pub(crate) fn ctor_is_withheld_at(&self, at: StructId, ctor_name: &str) -> bool {
        let Some(fs) = self.file_scope.as_ref() else {
            return false;
        };
        let Some(file) = self.visibility_file_of(at) else {
            return false;
        };
        !fs.visible_ctors[file].contains_key(ctor_name)
            && !fs.visible_ctors_qualified[file].contains_key(ctor_name)
    }

    /// The QUALIFIED-access analogue of [`Db::file_scoped_variant_ctor`] — resolves a variant name
    /// against `at`'s file including a prelude-named ctor (`Ast.Int`) that the BARE map omits. Used by
    /// the CDZ0214 withheld check and `is_abstract_type_at` to judge a `(. T A)` access: `Some(Ok(_))`
    /// means `A` is a visible constructor of `T` here (own type / wildcard / named import), so the
    /// qualified access is legitimate even when `A` collides with a prelude type name. Same tri-state as
    /// `file_scoped_variant_ctor`.
    pub(crate) fn file_scoped_variant_ctor_qualified(
        &self,
        at: StructId,
        name: &str,
    ) -> Option<Result<StructId, ()>> {
        let fs = self.file_scope.as_ref()?;
        let file = self.visibility_file_of(at)?;
        Some(
            fs.visible_ctors_qualified[file]
                .get(name)
                .copied()
                .ok_or(()),
        )
    }

    /// Whether the sum type whose DECLARATION OCCURRENCE is `decl` is ABSTRACT in the file containing
    /// node `at` — its handle is visible there but NONE of its constructors are (imported handle-only).
    /// A value of such a type may be named and held but not constructed, matched, stripped, or compared
    /// with a built-in `=`/`compare` in this file (the representation is the declaring module's private
    /// business). `false` for a concretely- or partially-imported type (≥1 ctor visible), a file's own
    /// type, a single-file program, or a node with no file (β-copied / prelude / synthesized).
    pub(crate) fn is_abstract_type_at(&self, at: StructId, decl: StructId) -> bool {
        let Some(fs) = self.file_scope.as_ref() else {
            return false;
        };
        let Some(file) = self.visibility_file_of(at) else {
            return false;
        };
        let Some(td) = self.type_decl_by_occ(decl) else {
            return false;
        };
        // The handle must be visible in this file (otherwise it is simply not this type here — a value
        // of it could not have been produced), and NO constructor of it visible (else it is concrete /
        // partial here, and comparison of the visible-ctor value is the module's own business).
        let handle_visible = fs.visible_types[file]
            .values()
            .any(|&synth| td.synth == Some(synth));
        if !handle_visible {
            return false;
        }
        // Consult the QUALIFIED ctor surface: a variant reachable only via `(. T A)` — e.g. a
        // prelude-named ctor like `Ast.Int`, omitted from the bare map — still makes the type CONCRETE
        // (not abstract) in this file. Using the bare map would wrongly call a type abstract when all its
        // visible constructors happen to collide with prelude names.
        !td.variants.iter().any(|v| {
            v.ctor
                .is_some_and(|c| fs.visible_ctors_qualified[file].values().any(|&vc| vc == c))
        })
    }

    /// Whether a package is linked (multi-file). When false, resolution is the flat single-file path.
    pub(crate) fn is_linked_package(&self) -> bool {
        self.file_scope.is_some()
    }

    /// The index of the FILE whose demux range contains node `id`, or `None` for a single-file program or
    /// a node in no file (prelude / β-copied / synthesized). Used to scope a per-MODULE well-formedness
    /// check (a duplicate declaration is per-file: two modules may each declare a type named `L`) — the
    /// same file identity the resolver uses for per-file name visibility.
    pub(crate) fn file_of(&self, id: StructId) -> Option<usize> {
        self.file_scope.as_ref()?.file_of(id)
    }

    /// The `link` PATH of file index `fi` (the module's source path, e.g. `foo/bar.cdz`) — what the
    /// `EmitTestsPerFile` build names each per-file test component artifact by, so `cdz test` can demux the
    /// N components back to their files. `None` for a single-file (unlinked) compile or an out-of-range
    /// index. Reads the same `FileSpan` table `file_of` indexes into.
    pub(crate) fn file_path(&self, fi: usize) -> Option<&str> {
        self.file_scope.as_ref()?.file_path(fi)
    }

    /// How many top-level defs across the WHOLE package share the value name `name` — the ambiguity
    /// count the β-copy fallback consults: 0 = unbound, 1 = unambiguous (resolve flat), >1 = ambiguous
    /// under a synthesized node whose file is unknown, so a copied reference to it DECLINES. Only
    /// meaningful for a linked package; a single-file program has one file, so it never exceeds 1 for a
    /// well-formed program (a duplicate def name there is a separate well-formedness concern).
    pub(crate) fn package_def_name_count(&self, name: &str) -> usize {
        self.defs.iter().filter(|d| d.name == name).count()
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

    /// The definitions marked `@test` (`strip_annotations`), as `defs` indices in DECLARATION ORDER — the
    /// set the `cdz test` build hoists into the test artifact as nullary boundary entries. Each `@test`
    /// marker is recorded by the annotated def's BODY occurrence, so this maps each back to its def via
    /// `def_index_by_body`. Declaration order (a `defs`-index sort) gives a deterministic, source-ordered
    /// test list. Empty for a program with no `@test` (the ordinary build — nothing hoisted). A `@test`
    /// whose def has PARAMETERS is still returned here; the emit path validates nullary-ness and rejects a
    /// parameterized test with an actionable diagnostic (a test takes no arguments).
    pub fn test_defs(&self) -> Vec<usize> {
        let mut idxs: Vec<usize> = self
            .tests
            .iter()
            .filter_map(|&body| self.def_index_by_body(body))
            .collect();
        idxs.sort_unstable();
        idxs.dedup();
        idxs
    }

    /// The `@tag("…")` string tags definition `def` carries, in annotation order (empty if it has no
    /// `@tag`, or if `def` is out of range / bodyless). Keyed by the def's BODY occurrence — the same
    /// identity `test_defs`/the inline sets use — so a tag recorded by `strip_annotations` against a def's
    /// body is found here by its `defs` index. Backs `cdz test --tag <tag>`: a test runs iff `tags_of`
    /// for its def contains the requested tag (AND-composed with the `--case` name filter). A tag is
    /// INDEPENDENT of `@test`, so this returns tags on any def, test or not.
    pub fn tags_of(&self, def: usize) -> &[String] {
        const NONE: &[String] = &[];
        self.defs
            .get(def)
            .and_then(|d| d.body)
            .and_then(|body| self.tags.get(&body))
            .map_or(NONE, Vec::as_slice)
    }

    /// The `@requires(pred)` PRECONDITION predicate occurrences definition `def` carries, in annotation
    /// order (empty if none, or `def` is out of range / bodyless). Keyed by the def's BODY occurrence, like
    /// `tags_of`. Each `StructId` is a predicate FORM over the def's params — the verification layer denotes
    /// it into a HOL obligation (Inc-b b4/b3). Backs the proof-guided-elision oracle's per-node precondition
    /// lookup.
    pub fn requires_of(&self, def: usize) -> &[StructId] {
        const NONE: &[StructId] = &[];
        self.defs
            .get(def)
            .and_then(|d| d.body)
            .and_then(|body| self.requires.get(&body))
            .map_or(NONE, Vec::as_slice)
    }

    /// The `@ensures(pred)` POSTCONDITION predicate occurrences definition `def` carries, in annotation
    /// order. Keyed by the def's BODY occurrence. Each `StructId` is a predicate FORM over the def's params
    /// and the implicit result binder `it` — denoted + discharged by the verification layer (Inc-b).
    pub fn ensures_of(&self, def: usize) -> &[StructId] {
        const NONE: &[StructId] = &[];
        self.defs
            .get(def)
            .and_then(|d| d.body)
            .and_then(|body| self.ensures.get(&body))
            .map_or(NONE, Vec::as_slice)
    }

    /// The `@invariant(pred)` DATA-TYPE-INVARIANT predicate the type declared at `type_occ` carries, if any
    /// — a predicate FORM over the value binder `it` (design §10). Keyed by the `(type …)` declaration
    /// occurrence (`TypeDecl::occ` — the type's nominal identity), NOT a def BODY occ, since `@invariant`
    /// annotates a type, not a def. `None` for a type with no `@invariant`. The DATA-level analogue of
    /// `requires_of`/`ensures_of`: the single source of "what is a valid value of this type", consumed by the
    /// verification layer (establish/preserve), the optimizer (data-level elision), and v-property-testing's
    /// `gen<T>` (the generation constraint).
    pub fn invariant_of(&self, type_occ: StructId) -> Option<StructId> {
        self.invariants.get(&type_occ).copied()
    }

    /// Every recorded `@invariant` predicate occurrence (across all annotated types), for whole-program
    /// validation — e.g. `collect_faults`' name-resolution of the predicate (a stray name → CDZ0101). Order
    /// is unspecified (a `HashMap` iteration); callers that need determinism sort.
    pub fn invariant_preds(&self) -> Vec<StructId> {
        self.invariants.values().copied().collect()
    }

    /// Whether the program declares ANY `@invariant` type — an O(1) check the establish-divert gate (in
    /// `lower_sum_new`) tests FIRST, before the per-construction `variant_owner_decl` (a type-inference
    /// query). A program with no `@invariant` (the overwhelming majority, incl. every non-verification test)
    /// then pays only this one hashmap-emptiness check per construction — NOT a second `scheme_of` on top of
    /// the newtype-erasure branch's. That matters on the layout reachability walk, which descends a
    /// `Core::SumNew` payload chain level-by-level: an extra `scheme_of` per level (on a pathological unbounded
    /// self-application chain) adds native recursion depth and overflows the compile worker's stack before the
    /// walk's own depth guard stops it. Gating on this keeps a no-`@invariant` program byte-identical to before.
    pub(crate) fn has_invariants(&self) -> bool {
        !self.invariants.is_empty()
    }

    /// Whether definition `def` is marked `@exhaustive` — a property test the runner drives over its ENTIRE
    /// finite input domain rather than by random sampling (`property-based-testing.md` §Exhaustive). Keyed
    /// by the def's BODY occurrence, the same identity `is_test`/`tags_of` use. Backs the `cdz test`
    /// runner's choice of exhaustive enumeration vs sampled trials. `false` for a def out of range /
    /// bodyless / not so annotated.
    pub fn is_exhaustive(&self, def: usize) -> bool {
        self.defs
            .get(def)
            .and_then(|d| d.body)
            .is_some_and(|body| self.exhaustive.contains(&body))
    }

    /// The index in [`defs`] of the def whose SIGNATURE occurrence is `sig` — the O(1) reverse backing
    /// `infer::def_of_param` (was a linear `defs.iter().position(|d| d.sig_occ == sig)` → O(N²) when
    /// called per parameter reference). Lazily builds + caches a `sig_occ → index` map, keyed alongside
    /// the `defs.len()` it was built at so a later synthesized-def push (effect specialization) triggers a
    /// rebuild (pushes are rare, so it is amortized). A def with a duplicated `sig_occ` cannot occur (each
    /// def has its own signature node), so no collision; FIRST-wins on the defensive off-chance.
    pub(crate) fn def_index_by_sig(&mut self, sig: StructId) -> Option<usize> {
        let stale = match &self.def_by_sig {
            Some((built_len, _)) => *built_len != self.defs.len(),
            None => true,
        };
        if stale {
            let mut map = crate::fxhash::FxHashMap::default();
            for (i, d) in self.defs.iter().enumerate() {
                map.entry(d.sig_occ).or_insert(i);
            }
            self.def_by_sig = Some((self.defs.len(), map));
        }
        self.def_by_sig.as_ref().unwrap().1.get(&sig).copied()
    }

    /// The index in [`defs`] of the def that node `id` IDENTIFIES — its `(def …)` form, its signature list,
    /// or its NAME atom — or `None` if `id` is not a def header node. The O(1) reverse backing
    /// `sidecar::def_identified_by` (was a linear `defs.iter().enumerate()` scan per query → O(N²) under
    /// `cdz query --where`'s per-match `TypeAt`). Lazily builds + caches the (≤3-per-def) header→index map,
    /// length-guarded to rebuild after a synth-def push (like [`def_index_by_sig`]).
    pub(crate) fn def_index_by_ident(&mut self, id: StructId) -> Option<usize> {
        let stale = match &self.def_by_ident {
            Some((built_len, _)) => *built_len != self.defs.len(),
            None => true,
        };
        if stale {
            let mut map = crate::fxhash::FxHashMap::default();
            for (i, d) in self.defs.iter().enumerate() {
                // The three header nodes: the signature list, its NAME atom (first child), and the
                // enclosing `(def …)` form (the signature's parent).
                map.entry(d.sig_occ).or_insert(i);
                if let Struct::List(kids) = self.ast.get(d.sig_occ)
                    && let Some(&name_occ) = kids.first()
                {
                    map.entry(name_occ).or_insert(i);
                }
                if let Some(form) = self.parent_of(d.sig_occ) {
                    map.entry(form).or_insert(i);
                }
            }
            self.def_by_ident = Some((self.defs.len(), map));
        }
        self.def_by_ident.as_ref().unwrap().1.get(&id).copied()
    }

    /// Register a recursive DO-LOCAL function found in a β-REDUCED (copied) body as an INTERNAL callable
    /// def, so its recursive call lowers to a `Core::Call` — the copy-time counterpart of the load-time
    /// `modules::register_do_local_callables`. When a HELPER whose body carries a do-local `(def (fac n)
    /// …)` is inlined at its call site, `beta_reduce` COPIES the def (fresh `StructId`s), so the copied
    /// recursive self-call resolves (via `do_local_binds`, forward-inclusive for functions) to the COPY's
    /// lambda — whose body is NOT in `def_by_body`, so `callee_def_index` misses it and the call declines
    /// "needs runtime specialization". Walking the reduced `root` for do-block FUNCTION defs and
    /// registering each (keyed by its fresh body) closes the gap: the load-time registration is by the
    /// ORIGINAL body, this one by the COPY, and a reference resolves to whichever body is in scope.
    ///
    /// Idempotent + bounded: `apply_lambda` memoizes each call site's reduction, so a given copy's defs
    /// register once; a def whose body is already in `def_by_body` (a re-walk, or a top-level def) is
    /// skipped. INTERNAL discipline (like the load-time path): kept OUT of `def_name_index` (the name
    /// resolves by lexical scope) and not an export / unused-warning target.
    pub fn register_reduced_callables(&mut self, root: StructId) {
        // A `(do …)` block's DIRECT `(def (f p…) BODY)` children with ≥1 param are the recursive-lowering
        // candidates. Walk the subtree at `root` (a shallow β-copy) collecting them first (an immutable
        // read), then register — so the mutation does not disturb the walk.
        let mut pending: Vec<(StructId, Vec<StructId>, StructId)> = Vec::new();
        self.collect_reduced_callables(root, &mut pending);
        for (sig, params, body) in pending {
            if self.def_by_body.contains_key(&body) {
                continue; // already registered (a re-walk, or a shared/uncopied body)
            }
            let name = match self.ast.get(sig) {
                Struct::List(kids) => kids
                    .first()
                    .and_then(|&c| self.ast.as_name(c))
                    .unwrap_or("")
                    .to_string(),
                _ => continue,
            };
            let idx = self.defs.len();
            self.def_by_body.insert(body, idx);
            self.defs.push(Def {
                name,
                sig_occ: sig,
                params,
                body: Some(body),
                internal: true,
            });
        }
    }

    /// Gather the do-block FUNCTION defs `(def (f p…) BODY)` (≥1 param) reachable under `node` as
    /// `(sig-occ, param-occs, body-occ)` — the recursive-descent helper of [`register_reduced_callables`].
    /// A def is a candidate only when its PARENT is a `(do …)` block (a genuine do-local declaration, the
    /// same shape the load-time scan registers), so a `(def …)` in any other position is not mistaken for
    /// one. Descends every child to reach a do-block nested at any depth in the copied body.
    fn collect_reduced_callables(
        &mut self,
        node: StructId,
        out: &mut Vec<(StructId, Vec<StructId>, StructId)>,
    ) {
        #[cfg(test)]
        COLLECT_REDUCED_CALLABLES_VISITS.with(|c| c.set(c.get() + 1));
        // Already fully walked (by an earlier `register_reduced_callables` on an overlapping reduced term)?
        // A node's structure is immutable once built, so a re-walk yields exactly the same candidates — skip
        // it. Without this, N β-reductions of a progressively-larger nested term each re-walked the whole
        // O(N)-deep subtree = O(N²) (a `((. d op0) ((. d op1) … acc))` dictionary-projection chain: a
        // depth-800 chain's callable scan alone was ~2.5M node-visits). `insert` returns false if present.
        if !self.reduced_callable_walked.insert(node) {
            return;
        }
        // Is `node` a do-block? If so, register each direct `(def (f p…) BODY)` child with params.
        if let Some(forms) = self.ast.as_form(node, "do") {
            for &form in forms {
                if let Some(tail) = self.ast.as_form(form, "def")
                    && let (Some(&sig), Some(&body)) = (tail.first(), tail.get(1))
                    && let Struct::List(children) = self.ast.get(sig)
                    && children.len() >= 2
                {
                    let params: Vec<StructId> = children[1..].to_vec();
                    out.push((sig, params, body));
                }
            }
        }
        // Descend every child (a do-block may be nested inside an if-branch, a let body, a def body copied
        // into this reduced term, etc.).
        if let Struct::List(children) = self.ast.get(node) {
            for c in children.clone() {
                self.collect_reduced_callables(c, out);
            }
        }
    }

    /// Normalize a decoded sum `(decl, name, args)` into its `Ty` — the ONE place the `Sum`↔`Nominal`
    /// decision + generic template substitution lives, so BOTH decode paths agree: `resolve::decode_ty`
    /// (a `(Sum …)` wire node) and `eval::reduce_sum_ctor` (a generic ctor `(Box Int64)` type
    /// application, which builds the `Ty` directly without a wire round-trip). An ERASABLE newtype decl
    /// has a stored inner TEMPLATE (params as `Ty::Var(i)`); substitute `args` positionally to get the
    /// inner at this instantiation and return `Ty::Nominal`. A non-erasable decl returns the boxed
    /// `Ty::Sum`. Pure over `&self` (a map lookup + a structural substitution) — no reduction.
    pub(crate) fn normalize_sum(&self, decl: StructId, args: Vec<crate::ty::Ty>) -> crate::ty::Ty {
        if let Some(template) = self.newtype_inner.get(&decl) {
            let inner = subst_template_vars(template, &args);
            return crate::ty::Ty::Nominal {
                decl,
                // `args` is the nominal's IDENTITY axis (like `Ty::Sum::args`) — `Box Int64` vs `Box
                // Bool` differ here. `inner` is the derived machine-rep hint (the template with `args`
                // substituted); it is never compared, so a recursive nominal's divergent inner is
                // harmless. Both come from the same `args`, kept consistent.
                args: args.into(),
                inner: std::rc::Rc::new(inner),
            };
        }
        crate::ty::Ty::Sum {
            decl,
            args: args.into(),
        }
    }

    /// The synthesized SUM RECORD of the type named `name`, if one is declared — how a `(type NAME …)`
    /// name resolves, exactly parallel to `def_by_name`. Returns the record occurrence a bare `NAME`
    /// denotes (a `Ref` to it), so `Option`, `Option.Some`, and `(: x Option)` take the ordinary
    /// member-access / `(meta t)` paths. First-wins on the name (like `def_by_name`); the record's own
    /// `(meta t)` occurrence — NOT the name — is the type's IDENTITY, so two same-named declarations are
    /// still distinct types (unlike a name-keyed map, which would clobber). A duplicate type NAME in one
    /// scope is a well-formedness concern for the checker, not this index.
    pub fn type_decl_by_name(&self, name: &str) -> Option<StructId> {
        // O(1) via `type_decl_index` (built at load). This was a linear `type_decls.iter().find` per call
        // — O(types) — and `resolve_name` calls it for MANY references, so a program with N sum types was
        // O(N²). First-declared-wins matches the old scan; the returned synth record is identical.
        self.type_decl_index.get(name).copied()
    }

    /// The synthesized RECORD occurrence of the nested `(module …)` at declaration occurrence `occ`, if
    /// `occ` is a known module declaration. How a do-local module NAME resolves — `do_local_binds` holds
    /// the `(module …)` form (the declaration occurrence) and returns a `Ref` to this record, so `(. m
    /// field)` is ordinary member access. `None` if `occ` names no module (a `(module …)` that failed to
    /// scan, or a non-module occurrence).
    pub fn module_synth_by_occ(&self, occ: StructId) -> Option<StructId> {
        self.modules
            .iter()
            .find(|m| m.occ == occ)
            .and_then(|m| m.synth)
    }

    /// The SYNTHESIZED record a TOP-LEVEL module NAME resolves to — the module analogue of
    /// [`effect_decl_by_name`]/[`type_decl_by_name`]. A `(module m …)` that is a DIRECT child of the
    /// program root (a sibling of the top-level defs, `(do (module m …) (def (main) …) (export main))`)
    /// binds `m` PROGRAM-WIDE, exactly as a top-level def/type/effect does — so a reference from any
    /// top-level def's body resolves it and `(. m field)` is ordinary member access. A do-LOCAL module
    /// (nested inside a def body's `(do …)`) is DELIBERATELY excluded: it is lexically scoped and resolved
    /// by `resolve::do_local_binds`'s parent walk, so surfacing it here would leak it program-wide and
    /// break its shadowing. `None` if no top-level module of that name is declared. First-declared wins (a
    /// duplicate top-level module name is a separate well-formedness concern).
    pub fn top_level_module_by_name(&self, name: &str) -> Option<StructId> {
        self.modules.iter().find_map(|m| {
            (m.name == name && self.parent_of(m.occ) == Some(self.ast.root))
                .then_some(m.synth)
                .flatten()
        })
    }

    /// The DECLARATION occurrence (`(module …)` form) whose synthesized record IS `record`, if any — the
    /// reverse of [`module_synth_by_occ`]. A module's members are mutually visible in each other's bodies,
    /// and the synthesized record is the join point every member body's parent chain ascends through (a
    /// member's synth field lambda reuses the member body, so the body's ancestors lead up to the record);
    /// `resolve::binder_in` uses this to recognize the record as a scope and resolve a bare sibling
    /// reference against the module's `(def …)` members. `None` if `record` is not a module's synth record
    /// (a sum/effect synth record, a user record, or any other node).
    pub fn module_by_synth_record(&self, record: StructId) -> Option<StructId> {
        self.modules
            .iter()
            .find(|m| m.synth == Some(record))
            .map(|m| m.occ)
    }

    /// The NAME of the module whose SYNTHESIZED record is `record` (`m` for `(module m …)`), or `None` if
    /// `record` is not a module's synth record. The name-returning companion of [`Db::module_by_synth_record`]
    /// — used by `member_category` to name a USER-module member miss "the `m` module has no member `k`"
    /// (matching the prelude-module arm), instead of leaking the internal "record has no field `k`" spelling.
    pub fn module_name_by_synth_record(&self, record: StructId) -> Option<&str> {
        self.modules
            .iter()
            .find(|m| m.synth == Some(record))
            .map(|m| m.name.as_str())
    }

    /// The constructor-field occurrence a BARE variant name denotes — `NLit` for `(type Node (NLit …) …)`,
    /// the same field a qualified `(. Node NLit)` projects. A nullary variant used as a VALUE may be
    /// written bare (`NNil`, not `(. Node NNil)` — `core-semantics.md` §A Sum Type Constructor Is A
    /// Single-Arity Function), and a payload variant bare-applied (`(NLit 5)`); both resolve here to the
    /// ctor field, then take the ordinary application/`(meta variant)` paths. The BUILT-IN prelude sums
    /// (`Some`/`None`/`Ok`/`Err`) bind their bare variant names in the prelude map at load; this is the
    /// same binding for a USER `(type …)` declaration, resolved generically (a scan of `type_decls`, no
    /// name special-case). FIRST-WINS across declarations: if two sums declare a same-named variant a
    /// bare reference takes the first-declared, and a qualified `(. Type Variant)` disambiguates — exactly
    /// how `type_decl_by_name` / `def_by_name` resolve a shared name. `None` if no declared sum has a
    /// variant named `name`.
    pub fn variant_ctor_by_name(&self, name: &str) -> Option<StructId> {
        // O(1) via `variant_ctor_index` (built at load from each variant's synthesized `ctor`). This was
        // an O(variants) scan of `type_decls` + `variant_ctor_field` per call — O(N²) for an N-arm match
        // over one N-variant sum. FIRST-declared-wins is baked into the index build, matching the old
        // `type_decls`-order scan; the returned occurrence is identical to the old `variant_ctor_field`.
        self.variant_ctor_index.get(name).copied()
    }

    /// The ctor of a bare variant whose name COLLIDES with a prelude TYPE/MODULE name (`Int`/`List`/…) —
    /// the ones `variant_ctor_by_name` deliberately omits (the `9f326a2d` skip) so bare `Int` stays the
    /// width constructor everywhere it means the width type. `resolve_name` consults THIS only in
    /// application-HEAD position on a genuine USER node (a real `(Int 42)` value construct), where the
    /// user's variant SHADOWS the colliding prelude name (operator ruling) — the construct-position
    /// analogue of the same-name-newtype head-position rule and the match-pattern remap (`85faf395`).
    /// `None` if no declared sum has a prelude-colliding variant named `name`. First-declared wins.
    pub fn prelude_colliding_variant_ctor(&self, name: &str) -> Option<StructId> {
        self.prelude_colliding_variant_ctor_index.get(name).copied()
    }

    /// Whether `id` lies inside a TYPE-EXPRESSION subtree (a variant payload, an annotation type slot, an
    /// effect-op arrow). The construct-position variant shadow (`resolve_name` step 3d) checks this and
    /// falls through for a type-expression head, so `(List Ast)` in `(type Ast … (List (List Ast)))` stays
    /// the List TYPE constructor rather than being diverted to a `List` variant construction. See
    /// [`Db::type_expr_nodes`].
    pub fn is_type_expr_node(&self, id: StructId) -> bool {
        self.type_expr_nodes.contains(&id)
    }

    /// The variant-CONSTRUCTOR occurrence for a SAME-NAME NEWTYPE — a declaration `(type UserId (UserId
    /// …))` whose SINGLE variant's name IS the type name. `Some(ctor)` only for that exact shape; `None`
    /// for a multi-variant sum, or one whose variant name differs from the type name. This is what lets
    /// the ONE name `UserId` mean the CONSTRUCTOR in application/pattern-HEAD position (`(UserId 42)`,
    /// `(UserId n)`) while `type_decl_by_name` keeps it the TYPE everywhere else (`(: x UserId)`): the
    /// resolver picks between them by POSITION (the head-position dispatch `head_ctor` already uses for
    /// `list`/`tuple`), never by a name special-case. Cadenza's one namespace makes the two meanings
    /// collide otherwise (the type decl shadows the bare variant, so the ctor is unreachable by name).
    pub fn same_name_newtype_ctor(&self, name: &str) -> Option<StructId> {
        // O(1) via the load-time index (was a linear `type_decls.iter().find` → O(N²) when the linked
        // resolve path probes it per type-name reference in an N-type package).
        self.same_name_newtype_ctor_index.get(name).copied()
    }

    /// Whether `name` is a same-name ctor of a MONOMORPHIC sum (no type params). `resolve_name` uses this
    /// to fire the head-position ctor rule on a SYNTH node (a β-copied `(Meters a)` inlined at a call site)
    /// — safe only for a monomorphic sum, which (unlike a generic one) has no `sum_applied` synth type-expr
    /// that must stay the type. See the FACE-B block in `resolve_name`.
    pub fn same_name_monomorphic_ctor(&self, name: &str) -> bool {
        self.same_name_monomorphic_ctor_index.contains(name)
    }

    /// The `(index, TypeDecl)` of the sum whose DECLARATION OCCURRENCE is `occ` — the reverse of the
    /// nominal identity a `Ty::Sum { decl }` carries. Used by the escape renderer to recover a sum's
    /// variant names + payload types from its type-value. `None` if `occ` names no declaration.
    /// A render-time [`crate::ty::NameCtx`] over this db's declared types.
    pub fn name_ctx(&self) -> crate::ty::NameCtx<'_> {
        crate::ty::NameCtx::new(&self.type_decls)
    }

    pub fn type_decl_by_occ(&self, occ: StructId) -> Option<&TypeDecl> {
        // O(1) via the load-time occ→index map; fall back to the scan for an occ minted after load (none
        // today — `type_decls` is not mutated post-construction — but the fallback keeps this total).
        if let Some(&i) = self.type_decl_by_occ_index.get(&occ) {
            return self.type_decls.get(i);
        }
        self.type_decls.iter().find(|t| t.occ == occ)
    }

    /// The `TypeDecl` whose SYNTHESIZED record occurrence is `synth` — the reverse of `TypeDecl::synth`.
    /// Used to recover a type's variants from the handle a name resolves to (`type_decl_by_name` /
    /// `file_scoped_type` return the synth record, not the decl), e.g. to list an abstract type's
    /// constructors for a CDZ0214 diagnostic. `None` if no declaration synthesized that record.
    pub fn type_decl_by_synth(&self, synth: StructId) -> Option<&TypeDecl> {
        self.type_decls.iter().find(|t| t.synth == Some(synth))
    }

    /// The SYNTHESIZED record occurrence an effect NAME resolves to — the effect analogue of
    /// `type_decl_by_name`. `E` in value position denotes its record (fields = operation values), so
    /// `E.op` is ordinary member access. `None` if no effect of that name is declared.
    pub fn effect_decl_by_name(&self, name: &str) -> Option<StructId> {
        // O(1) via `effect_decl_index` (built at load) — was a linear `effect_decls.iter().find`.
        self.effect_decl_index.get(name).copied()
    }

    /// The [`EffectDecl`] whose DECLARATION OCCURRENCE is `occ` — the reverse of the identity an
    /// operation's `(meta effect-op)` channel carries, so a later pass recovers an effect's operation set
    /// from a projected op value. `None` if `occ` names no effect declaration.
    pub fn effect_decl_by_occ(&self, occ: StructId) -> Option<&EffectDecl> {
        self.effect_decls.iter().find(|e| e.occ == occ)
    }

    /// The [`EffectDecl`] whose SYNTHESIZED RECORD is `record` — the reverse of `effect_decl_by_name`'s
    /// `synth`, parallel to `module_by_synth_record` for modules. A bare effect NAME resolves to a `Ref`
    /// at this record, so a consumer holding the target occurrence (the highlight query) recognizes it as
    /// an effect without a name match. `None` if `record` is not an effect's synth record.
    pub fn effect_decl_by_synth(&self, record: StructId) -> Option<&EffectDecl> {
        self.effect_decls.iter().find(|e| e.synth == Some(record))
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

    /// The top-level items that are a bare NAME atom resolving to NOTHING — `(module m nonesuch …)`, the
    /// paren-less twin of the `(nonesuch)` APPLICATION that `unknown_top_forms` already rejects. A bare
    /// name in a top-level/module item position that names no definition, prelude entry, or type is an
    /// error under ANY reading of the grammar (whether or not a bare EXPRESSION is a legal top-level item
    /// — a pending design call — a bare expression referencing a binding that does not exist is broken),
    /// yet it was SILENTLY ACCEPTED: `head_name` returns `None` for an atom, so `unknown_top_forms` (which
    /// flags a LIST whose head is unbound) never sees it, and nothing else in the top-level scan claims a
    /// bare atom. Returned as `(name, occurrence)` for `collect_faults` to reject CDZ0101 exactly as the
    /// application form — the two should behave identically. The SAME resolution guards as
    /// `unknown_top_forms`: a name is unbound only if it is neither a grammar/keyword head nor resolves as
    /// a def / prelude entry / type. A LITERAL (`5`, `"s"` — `as_name` is `None`) or a BOUND bare name
    /// (`main`) is left to the pending bare-expression-legality ruling, not flagged here.
    pub fn unbound_bare_name_items(&self) -> Vec<(String, StructId)> {
        let mut out = Vec::new();
        for item in top_items(&self.ast) {
            if let Some(name) = self.ast.as_name(item)
                && !TOP_LEVEL_FORMS.contains(&name)
                && !crate::resolve::is_grammar_head(name)
                && self.def_by_name(name).is_none()
                && !self.prelude.contains_key(name)
                && self.type_decl_by_name(name).is_none()
            {
                out.push((name.to_string(), item));
            }
        }
        out
    }

    /// The TOP-LEVEL `(bind …)` peer-binding directives — the same scope `scan_effect_bindings` reads. A
    /// `(bind …)` is a top-level DIRECTIVE (like `def`/`export`/`effect`); a `(bind …)` list appearing
    /// NESTED (e.g. a handler arm for an effect operation named `bind` — `((bind (k) s body) …)`) is NOT a
    /// peer-binding and MUST NOT be scanned as one. The malformed-`(bind …)` diagnostic uses this so it
    /// matches the binding scanner's scope exactly (an arena-wide scan misreads a nested `(bind …)` arm as
    /// a malformed directive → a spurious CDZ0201 on a legal `bind` operation name).
    pub fn top_bind_forms(&self) -> Vec<StructId> {
        top_items(&self.ast)
            .into_iter()
            .filter(|&item| self.ast.as_form(item, "bind").is_some())
            .collect()
    }

    /// The top-level in-source `(world …)` TARGET-WORLD declaration, if the program declares one — the
    /// paren-form twin of an external KIND_WIT_WORLD artifact. Its parsed subtree is byte-identical to the
    /// artifact form: v-syntax's inline `world …` surface lowers through the SAME shared
    /// `world_schema_tree`/`wit_type_*` builders the artifact and rcdzc's own emit target (the landed
    /// cross-source identity gate), so `compile` codec-encodes this subtree VERBATIM into `db.wit_world`
    /// when no artifact is supplied (the artifact — a deliberate build input retargeting the world —
    /// OVERRIDES the in-source decl, mirroring effect-bind request-overrides-source). `world` is
    /// recognized-but-registers-nothing at the top level (`TOP_LEVEL_FORMS`, no `scan_top_level` branch —
    /// the `module-doc` precedent), so it never resolves as a value. Returns ALL such forms so the caller
    /// can DECLINE on more than one — a module targets AT MOST ONE world, so two `(world …)` decls is a
    /// structural error the compiler rejects rather than silently picking one (decline-don't-miscompile).
    pub fn top_world_forms(&self) -> Vec<StructId> {
        top_items(&self.ast)
            .into_iter()
            .filter(|&item| self.ast.as_form(item, "world").is_some())
            .collect()
    }

    /// Malformed elements of top-level `(export …)` clauses. An export clause names one or more
    /// definitions — `(export a)` / `(export a b …)` (the multi-name surface) — so EVERY tail element must
    /// be a bare NAME. A NON-name element — `(export (g x))`, `(export 5)`, `(export a 5)` — and an EMPTY
    /// `(export)` are otherwise SILENTLY DROPPED: the module scan records an `Export` only for the elements
    /// that `as_name`, dropping the rest with no diagnostic (and the scan's own comment defers the check
    /// HERE); an all-malformed clause registers nothing, `unknown_top_forms` skips it (its head IS the
    /// known `export`), and the program compiles as if the author never wrote it — the emit path then
    /// reports the misleading "nothing is public". Return `(clause_occ, bad_element_occ)` for each bad
    /// element (and `(clause_occ, None)` for an empty clause) so `collect_faults` can reject it with the
    /// actionable "an export names a definition: `(export g)`" (a scan-and-drop correctness hazard). One
    /// entry PER bad element so `(export 5 6)` reports both.
    pub fn malformed_exports(&self) -> Vec<(StructId, Option<StructId>)> {
        let mut out = Vec::new();
        for item in top_items(&self.ast) {
            let Some(tail) = self.ast.as_form(item, "export") else {
                continue;
            };
            if tail.is_empty() {
                out.push((item, None));
                continue;
            }
            for &s in tail {
                // A bare NAME `(export g)` is well-formed; so is a CONSTRUCTOR-EXPORT `(. T A)` / `(. T *)`
                // (the opaque-types surface — the handle plus a named constructor, or the wildcard). A
                // non-name element that is NOT a well-formed ctor-export is the malformed case.
                if self.ast.as_name(s).is_none() && !is_ctor_export_shape(&self.ast, s) {
                    out.push((item, Some(s)));
                }
            }
        }
        out
    }

    /// Each top-level DECLARATION-keyword form with ZERO operands — a bare `(def)` / `(type)` / `(effect)`
    /// — as `(keyword, form_occ)`. Such a form declares NOTHING (no name, no body/variants/ops) yet was
    /// SILENTLY ACCEPTED: it registers no `Def`/`TypeDecl`/`EffectDecl` (nothing to key on), so the
    /// per-declaration validation walks never see it, and `unknown_top_forms` skips it (its head IS a known
    /// keyword). The `(export)` empty case is handled by `malformed_exports`; this is its
    /// `def`/`type`/`effect` sibling. Returned for `collect_faults` to reject CDZ0201, each anchored at the
    /// bare form.
    pub fn empty_declaration_forms(&self) -> Vec<(&'static str, StructId)> {
        let mut out = Vec::new();
        for item in top_items(&self.ast) {
            for kw in ["def", "type", "effect"] {
                if self.ast.as_form(item, kw).is_some_and(<[_]>::is_empty) {
                    out.push((kw, item));
                }
            }
        }
        out
    }

    /// Each MALFORMED top-level `@`-annotation form that SURVIVED `strip_annotations`. `@` is the
    /// general-purpose annotation head `(@ <name> (def …))`, and `strip_annotations` rewrites every
    /// well-formed def-wrapping annotation IN PLACE to BE its inner def (the node's head becomes `def`, so
    /// it is no longer an `@` form here) — including an UNKNOWN name `(@ deprecated (def …))` (a
    /// transparent forward-compat marker, recorded in no policy set). Therefore ANY top-level `(@ …)` that
    /// REMAINS is one whose target was NOT a well-formed def to unwrap: a bare `(@)`, a name-only
    /// `(@ test)`, a non-form target `(@ test 5)`, a non-def list `(@ test (foo 1))`, or a malformed inner
    /// def `(@ test (def))`. Left alone each resolves as a misleading "unbound name `@` at the top level"
    /// (`@` is a recognized head, not a name) plus a phantom unbound-name for any def it hid. Returned for
    /// `collect_faults` to reject CDZ0201 naming the annotation shape.
    pub fn malformed_annotation_forms(&self) -> Vec<StructId> {
        let mut out = Vec::new();
        for item in top_items(&self.ast) {
            if self.ast.as_form(item, "@").is_some() {
                out.push(item);
            }
        }
        out
    }

    /// The `(@ (tag …) …)` annotation heads whose argument is NOT exactly one STRING — malformed `@tag`
    /// forms (`@tag(5)`, `@tag(foo)`, `@tag()`, `@tag("a" "b")`). Recorded by `strip_annotations` (which
    /// unwraps the wrapper in place, so the offending head can't be re-found post-strip). `collect_faults`
    /// rejects each: a `@tag` takes exactly one string literal; a malformed one is silently dropped (the
    /// tag is recorded nowhere), which would mask an author's mistake — so reject it instead.
    pub fn malformed_tag_forms(&self) -> &[StructId] {
        &self.malformed_tags
    }

    /// `@requires`/`@ensures` heads with not-exactly-one predicate argument (malformed by ARITY). Read by
    /// `collect_faults` to REJECT them (CDZ0201) — the same discipline `malformed_tag_forms` applies to
    /// `@tag`, so a shape-broken precondition/postcondition is an author-visible error, not a silent drop.
    pub fn malformed_verify_forms(&self) -> &[StructId] {
        &self.malformed_verify
    }

    /// Each well-formed-SHAPE constructor-export element `(. T A)` / `(. T *)` in a top-level `(export …)`
    /// clause, as `(element_occ, type_name_occ, ctor_name_occ)`. `element_occ` anchors a diagnostic at the
    /// `(. …)` element; `type_name_occ`/`ctor_name_occ` are the `T`/`A` atoms (the ctor is the reserved `*`
    /// for the wildcard). Used by `collect_faults` to SEMANTICALLY validate a ctor-export (the type is a
    /// real sum, the ctor is one of its variants) — the shape is already accepted by `malformed_exports`
    /// (`is_ctor_export_shape`), but the linker's `as_ctor_export` records it WITHOUT checking the type/ctor
    /// exist, so `(export (. T Nonesuch))` / `(export (. undeclared A))` was silently accepted.
    pub fn ctor_export_elements(&self) -> Vec<CtorExportElem> {
        let mut out = Vec::new();
        for item in top_items(&self.ast) {
            let Some(tail) = self.ast.as_form(item, "export") else {
                continue;
            };
            for &s in tail {
                if let Some(elem) = parse_ctor_export(&self.ast, s) {
                    out.push(elem);
                }
            }
        }
        out
    }
}

/// A parsed CONSTRUCTOR-EXPORT element — the type-handle name, the constructor key (`*` for the wildcard),
/// and the nodes a diagnostic/fix anchors to. Recognized in TWO surface forms (mirrors
/// `link::as_ctor_export`): the member-access LIST `(. T A)` / `(. T *)`, and the dotted WILDCARD ATOM
/// `T.*` (the reader keeps `*` — a reserved, non-identifier final member segment — UNsplit, so `T.*` reads
/// as one atom rather than desugaring to a `(. T *)` list the way a named ctor `T.A` does). For the list
/// the type/ctor are distinct nodes (`ty_anchor`/`ctor_anchor` point at each); for the atom there is only
/// the one atom node, so a type-name fix must RECONSTRUCT the whole atom (`is_atom` flags this).
pub struct CtorExportElem {
    /// The export element node — where a diagnostic anchors.
    pub elem: StructId,
    /// The type-handle name (`T` / `Colr`).
    pub ty_name: String,
    /// The constructor key — a variant name, or the reserved `*` for "all constructors".
    pub ctor_key: String,
    /// The node a type-name rename fix targets (list: the `T` node; atom: the whole atom).
    pub ty_anchor: StructId,
    /// The node a constructor-name rename fix targets (list: the `A` node; atom: the whole atom — unused,
    /// since an atom ctor-export is always the `*` wildcard, which skips the per-ctor check).
    pub ctor_anchor: StructId,
    /// True for the dotted-atom form `T.*`: a type-name fix replaces the WHOLE atom, so its replacement
    /// text must carry the `.*` tail (`Colr.*` -> `Color.*`), not the bare type name.
    pub is_atom: bool,
}

/// Parse `s` as a CONSTRUCTOR-EXPORT element, or `None` if it is not one (a bare name, an integer
/// projection, a malformed shape). See [`CtorExportElem`] for the two recognized surface forms.
fn parse_ctor_export(ast: &Arenas, s: StructId) -> Option<CtorExportElem> {
    // FORM 1 — the member-access LIST `(. T A)` / `(. T *)` (the explicit form, and what a named dotted
    // ctor `T.A` desugars to). BOTH segments must be names (the ctor may be the reserved `*`).
    if let Some(tail) = ast.as_form(s, ".") {
        if tail.len() != 2 {
            return None;
        }
        let ty = ast.as_name(tail[0])?;
        let key = ast.as_name(tail[1])?;
        return Some(CtorExportElem {
            elem: s,
            ty_name: ty.to_string(),
            ctor_key: key.to_string(),
            ty_anchor: tail[0],
            ctor_anchor: tail[1],
            is_atom: false,
        });
    }
    // FORM 2 — the dotted WILDCARD ATOM `T.*` (mirrors `link::as_ctor_export` FORM 2). Without this the
    // COMPILE-side validation never saw the wildcard atom: `db::exports`' scan pushed the literal `"T.*"`
    // as a bare export name that resolves to no definition, so `(export T.*)` fell through to the generic
    // "names no definition" (CDZ0101) instead of the ctor-export validator's sum-type / did-you-mean
    // diagnostics — while `link` already resolved the same atom via its own FORM 2. Split on the LAST `.`
    // so a namespaced type reads correctly.
    let name = ast.as_name(s)?;
    let (ty, key) = name.rsplit_once('.')?;
    if ty.is_empty() || key.is_empty() {
        return None;
    }
    Some(CtorExportElem {
        elem: s,
        ty_name: ty.to_string(),
        ctor_key: key.to_string(),
        ty_anchor: s,
        ctor_anchor: s,
        is_atom: true,
    })
}

/// Whether `s` is a well-formed CONSTRUCTOR-EXPORT element (the list `(. T A)` / `(. T *)` or the wildcard
/// atom `T.*`). Shared so `db::malformed_exports` treats a valid ctor-export as well-formed (not the
/// misleading "an export names a definition"), the `db::exports` scan routes it to the ctor-export
/// validator rather than the bare-export path, and the linker reads its visibility — all off one
/// recognizer ([`parse_ctor_export`]).
fn is_ctor_export_shape(ast: &Arenas, s: StructId) -> bool {
    parse_ctor_export(ast, s).is_some()
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
fn is_binding_candidate(
    ast: &Arenas,
    parent: &[Option<StructId>],
    module_records: &crate::fxhash::FxHashSet<StructId>,
    form: StructId,
) -> bool {
    // A nested module's SYNTHESIZED RECORD is a scope: its `(def …)` members are mutually visible in each
    // other's bodies (`resolve::binder_in`'s Case R / `module_sibling_binds`). The record is the join point
    // every member body's parent chain ascends through — a member's synth field lambda reuses the original
    // member body, so `parent_index` re-parents that body under the synth `fn`, and the field-pair's parent
    // is the record — so without the record as a candidate the scope-skip index would hop PAST it and Case R
    // would never fire, spuriously unbinding a bare sibling reference (the `is_binding_candidate` trap:
    // every binding form MUST be listed here). Checked first (an O(1) set probe); the record is a
    // `("record" …)` string-headed node with no head NAME, so the head/parent cases below never match it.
    if module_records.contains(&form) {
        return true;
    }
    // By head: a `let`/`fn`/`def` form, or a match-arm `(guard <binder> <cond>)` — the guard binds the
    // pattern's binder in scope for the guard COND (`binder_in`'s Case 5g / `guard_cond_binds`). Without
    // the guard as a candidate the scope-skip index would hop PAST it and Case 5g would never fire, so a
    // guard reference to its binder would be spuriously unbound.
    //
    // A `(module …)` DECLARATION form is a candidate too (`binder_in`'s Case R2 / `module_form_sibling_binds`):
    // its members are mutually visible in each other's bodies. An EXPORTED member's body is reparented under
    // its synth field lambda beneath the record (Case R lands there), but a PRIVATE member — no field built —
    // keeps its source parent, so its body's scope walk ascends through the `(module …)` form; without it as a
    // candidate the skip index would hop PAST it and a private member's sibling reference would spuriously
    // unbind (the `is_binding_candidate` trap: every binding form MUST be listed here).
    if let Some(h) = ast.head_name(form)
        && matches!(h, "let" | "fn" | "def" | "guard" | "module")
    {
        return true;
    }
    // A NESTED `(do …)` block whose forms include a `(def …)` declaration OR a `(module …)` declaration
    // binds that name for the following forms (`resolve::binder_in`'s Case 8 / `do_local_binds`). Without
    // the `do` as a candidate the scope-skip index would hop PAST it and the do-case would never fire, so
    // a reference to a do-local declaration would be spuriously unbound (the `is_binding_candidate` trap:
    // every binding form MUST be listed here). A `do` with NO declaration binds nothing, so it need not be
    // a candidate — a plain value-sequencing block stays on the fast non-binding spine. The PROGRAM ROOT
    // do is EXCLUDED: its defs are the top-level scan's (resolved file-scoped by name), not a lexical
    // do-scope (see `do_local_binds`).
    if form != ast.root
        && let Some(forms) = ast.as_form(form, "do")
        && forms
            .iter()
            .any(|&f| matches!(ast.head_name(f), Some("def") | Some("module")))
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
    // A `(def (NAME param…) body)`'s SIGNATURE LIST (parent is a `def`, `form` its first tail element) is a
    // scope: an EARLIER type-valued parameter is visible in a LATER parameter's annotation
    // (`(def (unbox (: t Type) (: b (Box t))) …)` — `(Box t)` reads `t`), the in-order signature scoping
    // `resolve::binder_in`'s Case 4b provides. Without the sig list as a candidate the scope-skip index
    // would hop PAST it and Case 4b would never fire, so `t` in `(Box t)` would be spuriously unbound (the
    // `is_binding_candidate` trap: every binding form MUST be listed here). The sig list is headed by NAME
    // (`unbox`), not a binding keyword, so the head case above never matched it.
    if let Some(tail) = ast.as_form(p, "def")
        && tail.first().copied() == Some(form)
        && matches!(ast.get(form), Struct::List(_))
    {
        return true; // the def signature list
    }
    if let Some(tail) = ast.as_form(p, "match") {
        // tail = [scrutinee, arm…]; `form` is an arm iff it is a tail element other than the scrutinee.
        if tail.first().copied() != Some(form) && tail.contains(&form) {
            return true;
        }
    }
    // A HANDLE ARM binds the operation's params + the state binder in its body (`binder_in`'s Case 7).
    // The arm's parent `p` is the handle's ARMS-LIST (the handle's 2nd tail element), so `form` is an arm
    // iff `p`'s own parent is a `(handle-internal …)` whose arms-list is `p` and `p` holds `form`. This
    // runs AFTER `effects::desugar_handles` (the scope index is built post-desugar), so the head is the
    // internal spelling. Without the arm as a candidate the scope-skip index would hop PAST it and Case 7
    // would never fire, so an arm-body reference to a param/state binder would be spuriously unbound (the
    // `is_binding_candidate` trap: every binding form MUST be listed here).
    if let Some(pp) = parent.get(p.0 as usize).copied().flatten()
        && ast
            .as_form(pp, crate::effects::HANDLE_INTERNAL)
            .and_then(|t| t.get(1).copied())
            == Some(p)
    {
        return true;
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
    module_records: &crate::fxhash::FxHashSet<StructId>,
) -> Vec<Option<(StructId, StructId)>> {
    let n = ast.structure.len();
    let mut skip: Vec<Option<(StructId, StructId)>> = vec![None; n];
    let mut done = vec![false; n];
    // MEMO of `is_binding_candidate(p)` per node — it is queried with the PARENT `p` once per CHILD of
    // `p`, so a wide parent (a `(do …)` block or a big `(record …)`/apply with N children) asked the same
    // question N times. `is_binding_candidate` is O(1) for most forms BUT O(children) for a `(do …)` (it
    // scans the block's forms for a `def`/`module`), so the repeat made a wide `do` O(N²). Caching the
    // per-parent verdict (a pure function of the node) collapses it to O(N) total. `None` = not yet asked.
    let mut cand: Vec<Option<bool>> = vec![None; n];
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
                    // Memoized per-parent verdict (see `cand`): the same `p` is asked once per child.
                    let is_cand = match cand[p.0 as usize] {
                        Some(v) => v,
                        None => {
                            let v = is_binding_candidate(ast, parent, module_records, p);
                            cand[p.0 as usize] = Some(v);
                            v
                        }
                    };
                    if is_cand {
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

/// Build the per-MATCH-ARM binder-name over-approximation (`Db::arm_pattern_names`): for every match arm
/// in the arena, the BINDER-POSITION name atoms of the arm's PATTERN region (all children except the body)
/// — every name EXCEPT the HEAD (child 0) of each list form (see `collect_binder_names`). A match arm is a
/// tail element (other than the scrutinee) of a `(match …)` — the same shape [`is_binding_candidate`]
/// recognizes for a match arm. The arm's binders (whole-scrutinee, variant/list/tuple/record/map/bin
/// payload, guard-cond) all sit in a NON-HEAD pattern position, so the collected set is a SUPERSET of the
/// names the arm can bind — an over-approximation that lets `binder_in` fast-reject a scope-walk hop whose
/// `name` is absent, without running the ~20-case arm cascade. Excluding heads drops constructor/keyword
/// names (`Some`/`None`/`tuple`/…), extending the fast-reject to references to a CONSTRUCTOR name (which
/// appear only as pattern heads) — the ctor-name references an all-atoms set would leave running the
/// cascade O(N²) times on a deeply-nested match.
///
/// The pattern region is "all children except the LAST": an unguarded arm is `(pattern body)` (child 0 =
/// pattern) and a guarded arm is `((guard pat cond) body)` (child 0 = the guard, holding both pat and
/// cond) — in both the body is the last child and every binder lives before it, so excluding only the
/// last child keeps all binders while dropping the body's references (which would otherwise bloat the set
/// and defeat the skip). One pass over the arena, each pattern node visited once (patterns are disjoint
/// from the bodies that hold inner arms) → O(nodes) total.
fn build_arm_pattern_names(
    ast: &Arenas,
    parent: &[Option<StructId>],
) -> crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashSet<String>> {
    // Collect the BINDER-POSITION name atoms of a pattern subtree into `acc` — every name EXCEPT the HEAD
    // (child 0) of each list form. In a match pattern the head is ALWAYS a constructor / keyword / field
    // name / segment type / map key (`Some` in `(Some x)`; `tuple`/`list`/`record`/`bin`/`map`/`guard`; the
    // field name `x` in `(record (x a))`; the seg type `u8` in `(bin (u8 n))`; the key `k` in `(map (k v))`)
    // and NEVER a binder — a binder is a bare whole-scrutinee name or a NON-head child (a ctor payload, a
    // tuple/list element, a record field VALUE, a seg-binder, a map VALUE, a rest binder). So skipping heads
    // keeps a safe SUPERSET of the arm's binders while EXCLUDING constructor names — extending the
    // `arm_cannot_bind` fast-reject to references to a CONSTRUCTOR name (`Some`/`None`), which appear only as
    // pattern heads and which the earlier all-atoms set still ran the cascade for (leaving ctor-name
    // references O(N²) on a deeply-nested match). A bare-name pattern (a whole-scrutinee binder, not a list)
    // is not a head, so it is kept as itself.
    fn collect_binder_names(
        ast: &Arenas,
        node: StructId,
        acc: &mut crate::fxhash::FxHashSet<String>,
    ) {
        if let Struct::List(children) = ast.get(node) {
            // Skip child 0 (the head — a ctor/keyword/field-name/seg-type/map-key, never a binder); recurse
            // the remaining children, where every binder lives.
            for &c in children.iter().skip(1) {
                collect_binder_names(ast, c, acc);
            }
        } else if let Some(n) = ast.as_name(node) {
            acc.insert(n.to_string());
        }
    }

    let mut out: crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashSet<String>> =
        crate::fxhash::FxHashMap::default();
    for i in 0..ast.structure.len() {
        let form = StructId(i as u32);
        // An arm is a tail element (≠ the scrutinee at tail position 0) of a `(match …)` — mirrors the
        // match-arm branch of `is_binding_candidate` so coverage matches the scope-skip candidates.
        let Some(p) = parent.get(i).copied().flatten() else {
            continue;
        };
        let Some(tail) = ast.as_form(p, "match") else {
            continue;
        };
        if tail.first().copied() == Some(form) || !tail.contains(&form) {
            continue; // the scrutinee, or not a tail element of this match
        }
        // The arm form's children; the pattern region is all but the last (the body).
        let Struct::List(children) = ast.get(form) else {
            continue;
        };
        if children.len() < 2 {
            continue; // a degenerate arm with no body — leave to the cascade
        }
        let mut names: crate::fxhash::FxHashSet<String> = crate::fxhash::FxHashSet::default();
        for &c in &children[..children.len() - 1] {
            collect_binder_names(ast, c, &mut names);
        }
        out.insert(form, names);
    }
    out
}

/// Index each let bindings-list's BARE binders by name → their ascending `(child-position, value-occ)`
/// pairs, so `resolve::last_binder_named` answers a lookup in O(log N) instead of an O(N) reverse prefix
/// scan (a wide accumulation `let` was O(N²) — see [`Db::let_binder_index`]).
///
/// A bindings-list is indexed ONLY when EVERY pair is a bare-name binding `(name V)` or an annotated-bare
/// `((: name T) V)` — the case whose `last_binder_named` branch returns `Ref { value: V }`. If any pair is
/// a DESTRUCTURING pattern (`((tuple …) V)`/`((Ctor …) V)`), the whole list is left OUT of the map, so the
/// resolver falls back to the exact linear walk for it (the fast path never replicates the pattern-binder
/// enumeration). This mirrors `last_binder_named`'s LHS peel (`(: pat T)` → `pat`, then the `as_name`
/// test) so an entry appears iff the linear branch would have returned a `Ref` for that pair.
fn build_let_binder_index(
    ast: &Arenas,
) -> crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>>> {
    let mut out: crate::fxhash::FxHashMap<
        StructId,
        crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>>,
    > = crate::fxhash::FxHashMap::default();
    for i in 0..ast.structure.len() {
        let form = StructId(i as u32);
        // A bindings-list is a `let`'s first tail element. Confirm via the parent `let` shape so an
        // unrelated headless list of 2-element sublists is not mistaken for one.
        let Some(tail) = ast.as_form(form, "let") else {
            continue;
        };
        let Some(&bindings_occ) = tail.first() else {
            continue;
        };
        let Struct::List(pairs) = ast.get(bindings_occ) else {
            continue;
        };
        let mut map: crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>> =
            crate::fxhash::FxHashMap::default();
        let mut all_bare = true;
        for (pos, &pair) in pairs.iter().enumerate() {
            let Struct::List(kv) = ast.get(pair) else {
                // A malformed pair (not a 2-list) binds nothing here; the linear walk skips it too.
                continue;
            };
            if kv.len() != 2 {
                continue;
            }
            // Peel an annotation `(: pat T)` to `pat`, exactly as `last_binder_named` does.
            let lhs = match ast.as_form(kv[0], ":") {
                Some(ann) if ann.len() == 2 => ann[0],
                _ => kv[0],
            };
            if let Some(n) = ast.as_name(lhs) {
                map.entry(n.to_string())
                    .or_default()
                    .push((pos as u32, kv[1]));
            } else {
                // A destructuring pattern LHS — this list needs the linear (SumPayload) path.
                all_bare = false;
                break;
            }
        }
        if all_bare && !map.is_empty() {
            out.insert(bindings_occ, map);
        }
    }
    out
}

/// Build the per-do-block declaration index (see [`Db::do_binder_index`]): for each `(do …)` form, map
/// each name a do-local `def`/`module` declares to the ascending positions (in the do's `forms` tail) of
/// the declaring forms. Scans each do-block's forms ONCE at load — replacing `binder_in`'s per-reference
/// O(forms) scans with an O(1) name lookup. The value is the declaring FORM's occurrence, so `resolve`
/// re-derives the exact `Ref`/`Lambda` via `do_def_binds` (no resolved binder is precomputed here).
fn build_do_binder_index(
    ast: &Arenas,
) -> crate::fxhash::FxHashMap<StructId, crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>>> {
    let mut out: crate::fxhash::FxHashMap<
        StructId,
        crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>>,
    > = crate::fxhash::FxHashMap::default();
    for i in 0..ast.structure.len() {
        let form = StructId(i as u32);
        // The root DO binds nothing lexically (`binder_in` returns early for it — a merged multi-file
        // root is not a do-scope), so it needs no index; skip it to match that early return. But a root
        // that is itself a `(module …)` (a bare `(module m … (export main))` program) IS a scope — its
        // members are mutually visible, resolved by `module_form_sibling_binds` (Case R2 / a PRIVATE
        // member's body, which never reaches the synth record). It must be indexed like any nested module,
        // else `do_forms_declaring` returns `None` for it and the resolver falls to an O(members) live scan
        // PER REFERENCE — O(N²) on a wide root module. So skip the root ONLY when it is a `do`.
        if form == ast.root && ast.as_form(form, "module").is_none() {
            continue;
        }
        // Index the members of a `(do …)` block AND a `(module …)` — both hold a sequence of `def`/`module`
        // declarations resolved by name, and both were scanned per-reference (`binder_in`'s do-scope and
        // `module_sibling_binds`). For a `do`, `forms` is the tail after the `do` head (position 0 = the
        // first form); for a `module`, it is the tail after NAME (`get(1..)`), so member position 0 is the
        // first member — the positions the respective resolvers use.
        let forms = if let Some(f) = ast.as_form(form, "do") {
            f
        } else if let Some(t) = ast.as_form(form, "module") {
            match t.get(1..) {
                Some(members) => members,
                None => continue,
            }
        } else {
            continue;
        };
        let mut map: crate::fxhash::FxHashMap<String, Vec<(u32, StructId)>> =
            crate::fxhash::FxHashMap::default();
        for (pos, &f) in forms.iter().enumerate() {
            // A do-local `(def NAME …)` / `(def (NAME …) …)` declares NAME; a `(module NAME …)` declares
            // NAME. Record the name → (position, declaring-form). The exact `Ref`/`Lambda`/module-record
            // the reference resolves to is computed by `do_def_binds` / the module arm on lookup.
            let declared = ast.as_form(f, "def").and_then(|tail| {
                let sig = *tail.first()?;
                match ast.get(sig) {
                    // `(def x V)` — bare-name value declaration.
                    Struct::Atom(_) => ast.as_name(sig),
                    // `(def (NAME param…) …)` — the signature's first child is NAME.
                    Struct::List(children) => children.first().and_then(|&c| ast.as_name(c)),
                }
            });
            let declared = declared.or_else(|| {
                ast.as_form(f, "module")
                    .and_then(|t| t.first())
                    .and_then(|&n| ast.as_name(n))
            });
            if let Some(name) = declared {
                map.entry(name.to_string())
                    .or_default()
                    .push((pos as u32, f));
            }
        }
        // Insert even an EMPTY map: a load-time do/module scope that declares nothing is still INDEXED, so
        // `do_forms_declaring` gives an O(1) negative (`Some(&[])`) rather than returning `None` and forcing
        // the caller's live fallback scan. Only a scope appended AFTER load (a copied body) is absent → the
        // fallback. (The root do is skipped above — it binds nothing lexically.)
        out.insert(form, map);
    }
    out
}

/// The declarations `scan_top_level` gathers: definitions, export requests, sum/effect/module
/// declarations. A named tuple (factored out to keep the return type legible).
type TopScan = (
    Vec<Def>,
    Vec<Export>,
    Vec<TypeDecl>,
    Vec<EffectDecl>,
    Vec<ModuleDecl>,
);

/// Strip a LEADING `(doc …)` form from every `(def …)` in the arena, IN PLACE, and RETURN the doc text
/// captured off each def — keyed by the def's SIGNATURE occurrence (`children[1]`), the stable node id
/// both name lookup (`def_by_name` → `Def::sig_occ`) and body lookup (`def_index_by_body` → `sig_occ`)
/// converge on, so a doc query can reach the string from either the def NAME or a cursor OFFSET.
///
/// A definition — value, function, or nullary — may carry a documentation form immediately after its
/// name/signature that documents it and is NOT part of the value (`glossary`: a definition is "a value,
/// function, type", so the doc affordance cannot depend on which — `(def (f) (doc "…") body)` AND `(def
/// answer (doc "…") 42)` both bind ignoring the doc). Every consumer reads a def's body as `tail.get(1)`
/// (the element right after the signature), so an un-stripped `(doc …)` would be mis-read AS the body
/// ("unbound name doc"). This normalizes every `(def sig (doc …)… body)` to `(def sig body)` at load —
/// one place, so every def KIND (top-level, do-local, module member) is fixed uniformly. The def keeps
/// its `StructId` (only its child list shrinks); the orphaned doc node stays in the arena, unreferenced.
///
/// Conservative: only a `(doc …)`-HEADED form BETWEEN the signature and the LAST tail element is dropped
/// (the body is always last). A def with ≤1 tail element, or whose only post-sig element is the body, is
/// untouched — byte-identical to before for every doc-less program.
///
/// The captured doc is carried in the canonical representation (this column, keyed off the AST node),
/// NOT discarded as lexical trivia — and because it is stripped from the body it never lowers to Core, so
/// it cannot change the program's runtime meaning. Every def KIND can carry a `(doc …)` (the affordance
/// does not depend on whether the def is a value, function, or type), so every part of a program is
/// documentable.
//= spec/capabilities/agent-authoring.md#documentation-is-part-of-the-representation
//# Documentation attached to a definition MUST be carried in the canonical representation rather than discarded as lexical trivia.
//= spec/capabilities/agent-authoring.md#documentation-is-part-of-the-representation
//# Any definition MUST be able to carry documentation, so that every part of a program can be documented.
//= spec/capabilities/agent-authoring.md#documentation-is-machine-readable
//# Documentation MUST NOT change the runtime meaning of a program.
// The `(doc "…")` rides as an ordinary node of the binary AST attached to its def — the codec's generic
// encode/decode round-trips it like any list node, and this load-time pass captures it into the doc
// column (post-decode). So the AST carries documentation attached to a definition, per ast-encoding.
// (The COMMENT half of that section — a comment as a tree node — is ALSO realized now: a `//` comment
// reifies to a `(comment "text" <form>)` node that `strip_comments` peels; see there for its citations.)
//= spec/contracts/ast-encoding.md#the-tree-carries-comments-and-documentation
//# The abstract syntax tree MUST be able to carry documentation attached to a definition, as required by the agent-authoring capability.
fn strip_def_docs(ast: &mut Arenas) -> crate::fxhash::FxHashMap<StructId, String> {
    let mut docs: crate::fxhash::FxHashMap<StructId, String> = crate::fxhash::FxHashMap::default();
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        if ast.as_form(id, "def").is_none() {
            continue;
        }
        // The full child list `[def-head, sig, middle…, body]` — need the head to rebuild it. Nothing to
        // strip unless there is a middle region (a def of `[head, sig, body]` = 3 children is minimal).
        let Struct::List(children) = ast.get(id) else {
            continue;
        };
        if children.len() <= 3 {
            continue;
        }
        // Keep the head + signature and the body (last); drop any `(doc …)`-headed form between them. A
        // non-doc middle element is left in place (defensive — a malformed multi-body def is diagnosed
        // elsewhere, not silently reshaped here).
        let n = children.len();
        let head = children[0];
        let sig = children[1];
        let body = children[n - 1];
        let middle = children[2..n - 1].to_vec();
        let mut middle_kept: Vec<StructId> = Vec::with_capacity(middle.len());
        let mut doc_text: Option<String> = None;
        for m in middle {
            // A `(doc "text")` form — capture its FIRST string operand (keyed by the def's signature), then
            // drop it from the def body. A malformed doc (no string operand) captures nothing but is still
            // stripped, so the body reader is unaffected. First doc wins if a def somehow carries several.
            if let Some(tail) = ast.as_form(m, "doc") {
                if doc_text.is_none() {
                    doc_text = tail
                        .first()
                        .and_then(|&t| ast.as_str(t))
                        .map(str::to_string);
                }
                continue;
            }
            middle_kept.push(m);
        }
        if let Some(text) = doc_text {
            docs.insert(sig, text);
        }
        if middle_kept.len() + 3 == n {
            continue; // no doc form found — leave the def untouched (the body stayed last)
        }
        let mut kept = vec![head, sig];
        kept.extend(middle_kept);
        kept.push(body);
        ast.structure[i] = Struct::List(kept);
    }
    docs
}

/// Unwrap every `(const BINDER)` PARAMETER wrapper in a `def` signature or `fn` parameter list, in place,
/// and return the set of the stripped params' NAME occurrences — the EXPLICIT compile-time parameters
/// (`DESIGN-recursive-generic-monomorphization-rcdzc.md` Addendum 3). A `const` param is inlined + erased
/// at instantiation; `const` is a DECLARATION consumed by the specializer, not part of the binder shape
/// the resolver/typer walk — so it is removed here (BEFORE `scan_top_level` / resolution), exactly as
/// `strip_def_docs` removes a doc form, leaving every downstream reader a plain binder.
///
/// A `const` param may wrap a bare name `(const d)` or an annotated binder `(const (: d T))`; the inner
/// node replaces the wrapper in the parameter list, and its NAME occurrence (the bare name, or the `(: name
/// T)`'s first child) is recorded. Runs over ALL forms (a def OR an fn), so a lambda `(fn ((const d) …)
/// …)` marks its const params too. Non-`const` params are untouched. A `(const …)` in a NON-parameter
/// position (a value expression) is NOT a parameter and is left alone (only a direct child of a def
/// signature / fn param list is unwrapped).
fn strip_const_params(ast: &mut Arenas) -> crate::fxhash::FxHashSet<StructId> {
    let mut const_params: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    // The NAME occurrence of a (already-unwrapped) binder: a bare name is itself; a `(: name T)` binder is
    // its first child. Mirrors `param_name_occ` but over `&Arenas` (no `Db` yet at load).
    fn name_occ_of(ast: &Arenas, binder: StructId) -> StructId {
        if ast.as_name(binder).is_some() {
            return binder;
        }
        if let Some(tail) = ast.as_form(binder, ":")
            && let Some(&n) = tail.first()
        {
            return n;
        }
        binder
    }
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        // The parameter list to scan: a `def`'s SIGNATURE (its first tail element, `[NAME, p…]` — params
        // are indices 1..) or an `fn`'s PARAMETER LIST (its first tail element, `(p…)` — params are all).
        let (list_occ, first_param_ix) = if let Some(tail) = ast.as_form(id, "def") {
            match tail.first() {
                Some(&sig) if matches!(ast.get(sig), Struct::List(_)) => (sig, 1usize),
                _ => continue,
            }
        } else if let Some(tail) = ast.as_form(id, "fn") {
            match tail.first() {
                Some(&params) if matches!(ast.get(params), Struct::List(_)) => (params, 0usize),
                _ => continue,
            }
        } else {
            continue;
        };
        let Struct::List(children) = ast.get(list_occ) else {
            continue;
        };
        let children = children.clone();
        let mut rewritten = children.clone();
        let mut changed = false;
        for (ix, &child) in children.iter().enumerate() {
            if ix < first_param_ix {
                continue; // the def NAME at index 0 is not a parameter
            }
            // A `(const BINDER)` wrapper — exactly one operand. Unwrap to BINDER, record its name occ.
            if let Some(tail) = ast.as_form(child, "const")
                && tail.len() == 1
            {
                let binder = tail[0];
                const_params.insert(name_occ_of(ast, binder));
                rewritten[ix] = binder;
                changed = true;
            }
        }
        if changed {
            ast.structure[list_occ.0 as usize] = Struct::List(rewritten);
        }
    }
    const_params
}

/// The known annotation NAMES `strip_annotations` records into a policy set off a `(@ NAME (def …))`
/// wrapper — the single source of truth for "which annotations carry compiler SEMANTICS today". The set
/// is the inline-policy pair (`inline-never`/`inline-always`), the test markers (`test`/`exhaustive`), and
/// the verification-contract predicates (`requires`/`ensures`). The contract pair is CALL-STYLE — the
/// annotation head is an APPLICATION `@requires(pred)`/`@ensures(pred)` carrying a predicate argument, not
/// a bare `NAME` like the others (`strip_annotations` matches the application head against this list). An
/// annotation whose name is NOT one of these is still UNWRAPPED (the def takes effect) but recorded
/// nowhere: a transparent, inert marker. So a future `@deprecated`/`@lint`/… works as a no-op the day it is
/// written and gains meaning by joining this list.
pub(crate) const KNOWN_ANNOTATIONS: &[&str] = &[
    "inline-never",
    "inline-always",
    "test",
    "exhaustive",
    "requires",
    "ensures",
    "invariant",
];
/// The strippable annotations a definition carries, each a set of the annotated defs' BODY occurrences.
pub(crate) struct StrippedAnnotations {
    pub(crate) inline_never: crate::fxhash::FxHashSet<StructId>,
    pub(crate) inline_always: crate::fxhash::FxHashSet<StructId>,
    /// Definitions marked `@test` — a UNIT TEST the `cdz test` build hoists into the test artifact as a
    /// nullary export (`property-based-testing.md` sibling: a plain assertion test). Recorded by BODY occ
    /// like the inline sets; `Db::is_test`/`test_defs` read it. Empty for an ordinary (non-test) build.
    pub(crate) tests: crate::fxhash::FxHashSet<StructId>,
    /// The `@tag("…")` string tags each annotated def carries, keyed by BODY occ (like `tests`). A def
    /// may carry several tags; they accumulate in annotation order. Read by `Db::tags_of` to back
    /// `cdz test --tag`. See the `Db::tags` field for the surface + semantics.
    pub(crate) tags: crate::fxhash::FxHashMap<StructId, Vec<String>>,
    /// Definitions marked `@exhaustive` — a property test the `cdz test` runner drives over its ENTIRE
    /// finite input domain (every combination of its scalar parameters) rather than by random sampling, so
    /// a pass is a proof over the domain (`property-based-testing.md` §Exhaustive). Recorded by BODY occ
    /// like `tests`; `Db::is_exhaustive` reads it. `@exhaustive` implies the def is a property test but is
    /// recorded independently of `@test` (the runner treats an `@exhaustive` def as a test too).
    pub(crate) exhaustive: crate::fxhash::FxHashSet<StructId>,
    /// `(@ (tag …) …)` annotation heads whose argument is not exactly one STRING — a malformed `@tag`
    /// (`@tag(5)`, `@tag(foo)`, `@tag()`, `@tag("a" "b")`). Recorded by the offending `(tag …)` occurrence
    /// so `collect_faults` rejects it (rather than silently dropping the tag and masking the author error).
    pub(crate) malformed_tags: Vec<StructId>,
    /// The `@requires(pred)` PRECONDITION predicates each annotated def carries, keyed by BODY occ (like
    /// `tags`). A def may stack several (`@requires(> x 0) @requires(< x 100)`); they accumulate in
    /// annotation order (a conjunction). The stored `StructId` is the PREDICATE-form occurrence — a
    /// Cadenza expression over the def's params — which the verification layer (Inc-b b4) DENOTES into a
    /// HOL obligation `Term`. This is the `@requires`/`@ensures`→node channel the proof-guided-elision
    /// oracle (`discharged_no_overflow`) needs: a checked-arith node's precondition is looked up here.
    /// (This b4a slice RECORDS the predicates; the denotation + discharge are later b4 slices. Recording
    /// them is behavior-neutral — nothing yet reads these sets, so an annotated def compiles exactly as
    /// today, just no longer as an inert unknown marker.)
    pub(crate) requires: crate::fxhash::FxHashMap<StructId, Vec<StructId>>,
    /// The `@ensures(pred)` POSTCONDITION predicates each annotated def carries, keyed by BODY occ. The
    /// stored `StructId` is the predicate-form occurrence, a Cadenza expression over the def's params +
    /// the implicit result binder `it` (v-syntax's result-binding convention). Denoted + discharged in a
    /// later b4 slice; recorded here (behavior-neutral) as the surface half of the channel.
    pub(crate) ensures: crate::fxhash::FxHashMap<StructId, Vec<StructId>>,
    /// The `@invariant(pred)` DATA-TYPE-INVARIANT predicate each annotated `(type …)` declaration carries,
    /// keyed by the TYPE decl occ (not a def BODY occ — `@invariant` annotates a type, not a def). The stored
    /// `StructId` is the predicate-form occurrence, a Cadenza expression over the value binder `it`. Recorded
    /// here (behavior-neutral) as the surface half of the data-level channel (design §10); consumed by
    /// `Db::invariant_of`.
    pub(crate) invariants: crate::fxhash::FxHashMap<StructId, StructId>,
    /// `(@ (requires …) …)` / `(@ (ensures …) …)` / `(@ (invariant …) …)` heads whose argument is not exactly
    /// one predicate form — a malformed `@requires`/`@ensures`/`@invariant` (`@requires()`, `@invariant(a b)`).
    /// Recorded by the offending occurrence so `collect_faults` REJECTS it (CDZ0201), the same arity discipline
    /// `malformed_tags` applies to `@tag` — a silently-unrecorded predicate would mask the author error (b4a2).
    pub(crate) malformed_verify: Vec<StructId>,
}

/// Unwrap EVERY `@`-ANNOTATION on a definition — `(@ inline-never (def …))`, `(@ inline-always (def …))`,
/// `(@ test (def …))`, and any `(@ <other-name> (def …))` — in place, returning the sets of the
/// annotated defs' BODY occurrences for the names in [`KNOWN_ANNOTATIONS`]
/// (`DESIGN-recursive-generic-monomorphization-rcdzc.md` Addendum 4). `@` is the GENERAL-PURPOSE
/// annotation head (`(@ NAME FORM)`, from the ML `@name form` sigil); a KNOWN name records its def into a
/// policy set, an UNKNOWN name is unwrapped just the same but recorded nowhere (a transparent no-op marker,
/// so a future `@deprecated`/… works the day it is written). An annotation is a declaration a build phase
/// consumes (the emitter's inline policy, the `cdz test` hoist) — not part of the def shape every reader
/// walks — so it is removed here BEFORE `scan_top_level`, exactly as `strip_def_docs`/`strip_const_params`
/// remove their wrappers, leaving every downstream reader a plain `(def …)`.
///
/// The annotation NODE is rewritten IN PLACE to BE the inner `(def SIG BODY)` (it adopts the def's
/// children), so its `StructId` now identifies the def and every parent that already pointed at the
/// annotation needs no update — the inner def node is left orphaned (harmless; unreferenced). The
/// recorded key is the def's BODY occurrence (`def` tail index 1 = child index 2), the identity
/// `lower`/`layout` key on (`def_index_by_body`). An UNKNOWN annotation name (or a non-name head) whose
/// inner IS a def is STILL unwrapped — recorded in no policy set, a transparent no-op — so the def takes
/// effect; only its (unmodeled) name is ignored. An annotation around a NON-def (or a malformed def) is
/// left untouched (a well-formedness concern elsewhere). A SINGLE annotation per def is the modeled case
/// (as with the original inline-policy pass); stacking two known annotations on one def is not relied on.
/// Peel every `(comment "…" <form>)` wrapper to its inner `<form>`, IN PLACE (the wrapper node adopts the
/// inner form's children, ids unchanged), so the compiler sees THROUGH a comment exactly as it sees
/// through a `(doc …)`. A comment is semantically inert (self-hosting-surface.md §the tree carries
/// comments and documentation — "each a node the compiler sees through"); a leading `//` on a form reifies
/// (by the reader) to `(comment "text" <form>)`, and without this pass the top-level scan reads `comment`
/// as an unknown declaration head → the wrapped `def`/`type`/`export` is invisible ("unbound name
/// `comment`" + the def's name unbound). Stacked comments NEST — `// a` then `// b` on one form is
/// `(comment "a" (comment "b" <form>))` — so each wrapper is peeled to the INNERMOST non-comment form (the
/// intervening comment nodes are then orphaned, harmless like `strip_annotations`'s unwrapped inner). A
/// `(comment …)` of the wrong arity (not exactly `<string> <form>`) is left untouched (a malformed node a
/// later pass handles). Runs FIRST in `load_linked`, before the doc/const/annotation strips + the scan.
///
/// The `(comment "text" <form>)` this pass peels IS a comment carried as a NODE of the AST — the reader
/// reifies a `//` comment into it, wrapping (attached to) the form it annotates, and it is an ordinary
/// `Struct::List` so it lives in the stored BINARY form (the codec's generic encode/decode round-trips it
/// unchanged, exactly as it does the `(doc …)` node), not only in a textual rendering. That this pass can
/// find and peel a `(comment …)` at load is the proof the tree carries it.
///
/// `pub(crate)` so `compile.rs::link_inputs` can peel each package file's arena BEFORE `link` scans its
/// imports/exports (the link scan runs before `Db::load`'s own call to this, so a comment on an
/// `(import …)` would otherwise leave it wrapped + unrecognized).
//= spec/contracts/ast-encoding.md#the-tree-carries-comments-and-documentation
//# The abstract syntax tree MUST be able to carry a comment as a node of the tree, attached to the node it annotates, so that a comment is preserved in the stored binary form rather than only in a textual rendering.
//= spec/contracts/ast-encoding.md#the-tree-carries-comments-and-documentation
//# A comment or documentation carried by the tree MUST survive encoding and decoding unchanged.
// Peeling the wrapper at LOAD, before resolve/type/lower ever see the form, is what makes a comment
// SEMANTICALLY INERT: the compiler sees through it to the inner form, so a `(comment …)` changes neither
// the runtime meaning of a program nor the type its expressions are assigned — it is pure annotation.
//= spec/capabilities/agent-authoring.md#comments-are-semantically-inert
//# A comment MUST NOT change the runtime meaning of a program.
//= spec/capabilities/agent-authoring.md#comments-are-semantically-inert
//# A comment MUST NOT change the type a program's expressions are assigned.
pub(crate) fn strip_comments(ast: &mut Arenas) {
    // A reader-produced comment node the compiler peels through: a LEADING `(comment "text" form)` (a
    // `//`/`///` on its own line above `form`) OR a TRAILING `(comment-after "text" form)` (a `//` that
    // followed `form` on the same source line, e.g. `| Ctor(T) // note`). Both have the identical
    // `[<string>, <form>]` tail, so both peel to `form` by the same rule — the compiler never sees either.
    let comment_form = |ast: &Arenas, id: StructId| -> Option<StructId> {
        let tail = ast
            .as_form(id, "comment")
            .or_else(|| ast.as_form(id, "comment-after"))?;
        let (&text, &form) = (tail.first()?, tail.get(1)?);
        // The first tail element must be a STRING (the comment text); else it's not a reader comment node.
        matches!(ast.get(text), Struct::Atom(l) if matches!(ast.leaf(*l), Leaf::Str(_)))
            .then_some(form)
    };
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        if comment_form(ast, id).is_none() {
            continue;
        }
        // Follow the chain of nested comment nodes (leading and/or trailing, in any mix) down to the
        // first NON-comment form. A malformed comment stops the descent (left for a well-formedness pass).
        let mut inner = id;
        while let Some(form) = comment_form(ast, inner) {
            inner = form;
        }
        // `inner` is the innermost non-comment form (or `id` itself if the chain was malformed at the top —
        // then this is a no-op copy). Rewrite the OUTER comment node to BE that form (adopt its children),
        // so its `StructId` now identifies the form and every parent pointing at the comment needs no update.
        if inner != id {
            let entry = ast.get(inner).clone();
            ast.structure[i] = entry;
        }
    }
}

fn strip_annotations(ast: &mut Arenas) -> StrippedAnnotations {
    let mut inline_never: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    let mut inline_always: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    let mut tests: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    let mut tags: crate::fxhash::FxHashMap<StructId, Vec<String>> =
        crate::fxhash::FxHashMap::default();
    let mut exhaustive: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    // `(@ (tag …) …)` heads whose argument is not exactly one STRING — malformed tag annotations to reject.
    let mut malformed_tags: Vec<StructId> = Vec::new();
    let mut requires: crate::fxhash::FxHashMap<StructId, Vec<StructId>> =
        crate::fxhash::FxHashMap::default();
    let mut ensures: crate::fxhash::FxHashMap<StructId, Vec<StructId>> =
        crate::fxhash::FxHashMap::default();
    let mut invariants: crate::fxhash::FxHashMap<StructId, StructId> =
        crate::fxhash::FxHashMap::default();
    let mut malformed_verify: Vec<StructId> = Vec::new();
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        // `(@ NAME INNER)` — the annotation head `@`, its name, and the annotated form. Only a known
        // annotation name is consumed here; every other `@` annotation is left in place.
        let Some(tail) = ast.as_form(id, "@") else {
            continue;
        };
        let (Some(&name_occ), Some(&inner)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        // The annotation NAME, if it is a bare name at all. A KNOWN name (in `KNOWN_ANNOTATIONS`) records
        // its def into a policy set below; an UNKNOWN name is TRANSPARENT — the wrapper is still unwrapped
        // to the inner def (so the def registers and resolves), the name simply ignored. This is the
        // `@`-sigil's advertised extensibility (`7221f7bc`: "future annotations `@deprecated`, `@test` layer
        // in with no new lexer/parser/resolver rules" / "leaves other annotations in place") — "in place"
        // means the annotated FORM still takes effect, NOT that the `(@ …)` node survives to resolve (where
        // `@` has no declaration arm → the whole def would be DROPPED with a misleading "unbound name `@`"
        // plus a phantom unbound-name for the def it wrapped). A future annotation that needs real semantics
        // adds its name to `KNOWN_ANNOTATIONS` + a set here; until then it is an inert, unwrapped marker.
        let name = ast.as_name(name_occ).map(str::to_string);
        // A CALL-STYLE annotation carries an ARGUMENT: `@tag("slow")` reifies to `(@ (tag "slow") def)`,
        // so the NAME position is the APPLICATION `(tag "slow")` — a form, not a bare name (`as_name`
        // above is `None`). Read it as `(HEAD ARG…)`: `@tag("…")` is the modeled call-style annotation,
        // its head `tag` + a single STRING argument the tag text. A def may stack several
        // (`@tag("slow") @tag("net")`), each recorded below. Any other application-name annotation is an
        // inert unknown marker (unwrapped, recorded nowhere), exactly like an unknown bare name.
        let tag_app = ast.as_form(name_occ, "tag");
        let tag_arg: Option<String> = tag_app.and_then(|app_tail| {
            match app_tail {
                [only] => ast.as_str(*only).map(str::to_string),
                _ => None, // `@tag` with not-exactly-one-arg (or a non-string arg) — not a modeled tag
            }
        });
        // `@requires(pred)` / `@ensures(pred)` reify like `@tag`, but their ARGUMENT is a PREDICATE FORM
        // (a Cadenza expression over the def's params, and — for `@ensures` — the result binder `it`), not
        // a string. `@requires(> x 0)` → `(@ (requires (> x 0)) def)`, so the name position is the
        // application `(requires (> x 0))` and its single tail element is the predicate occurrence. Record
        // that `StructId`; the verification layer denotes it into a HOL obligation (a later b4 slice).
        //
        // ARITY validation (b4a2): a `@requires`/`@ensures` with NOT-exactly-one argument (`@requires()`,
        // `@requires(a b)`) is a pure SHAPE error `strip_annotations` can see — exactly like `@tag`
        // validates one-string. Record the offending occurrence in `malformed_verify` so `collect_faults`
        // REJECTS it (CDZ0201), rather than silently recording no predicate (which would mask the author
        // error, exactly the `@tag` masking bug). NAME-resolution / boolean-typedness of the predicate is
        // NOT checked here — that needs the def's param scope + the `it` binder, so it is deferred to the
        // denotation pass (b4c), which reports an unbound name at the annotation span.
        let requires_app = ast.as_form(name_occ, "requires");
        let ensures_app = ast.as_form(name_occ, "ensures");
        let requires_pred: Option<StructId> = requires_app.and_then(|t| match t {
            [only] => Some(*only),
            _ => None,
        });
        let ensures_pred: Option<StructId> = ensures_app.and_then(|t| match t {
            [only] => Some(*only),
            _ => None,
        });
        // A `(requires …)` / `(ensures …)` head with not-exactly-one arg → malformed (recorded below,
        // once we know the inner is a def, mirroring the `@tag` malformed discipline).
        let malformed_verify_here = (requires_app.is_some() && requires_pred.is_none())
            || (ensures_app.is_some() && ensures_pred.is_none());
        // `@invariant(pred)` — the DATA-level family member (design §10). UNLIKE `@requires`/`@ensures`, it
        // annotates a `(type …)` DECLARATION, not a `(def …)`. So it must be handled HERE, before the
        // def-only path below `continue`s on a non-def inner (which would otherwise route a `@invariant (type
        // …)` to the "annotation wraps no definition" CDZ0201). `@invariant(> (len it) 0)` reifies to
        // `(@ (invariant (> (len it) 0)) (type …))`, so the name position is the application `(invariant …)`
        // with one predicate tail element (over the value binder `it`). Record it keyed by the TYPE decl occ,
        // then UNWRAP the wrapper to the type decl (adopt its children) so the type still takes effect. Arity
        // is validated like `@requires`/`@ensures` (b4a2): not-exactly-one arg → `malformed_verify` (CDZ0201).
        let invariant_app = ast.as_form(name_occ, "invariant");
        if let Some(app_tail) = invariant_app {
            // Only meaningful when the inner is a `(type …)` declaration (the `@invariant` annotand). A
            // `@invariant` on a NON-type is left to the "wraps no definition" path below (not a modeled shape).
            if ast.as_form(inner, "type").is_some() {
                match app_tail {
                    [only] => {
                        // Key by `id` (the WRAPPER slot) — NOT `inner`. The unwrap below overwrites slot `id`
                        // with the type's children, so post-strip the `(type …)` DECLARATION lives at `id`,
                        // and `scan_top_level` computes `TypeDecl::occ = id`. Keying by `inner` would miss
                        // (`invariant_of(TypeDecl::occ)` looks up `id`).
                        invariants.insert(id, *only);
                    }
                    // not-exactly-one predicate arg → malformed (arity), rejected in `collect_faults`.
                    _ => malformed_verify.push(name_occ),
                }
                // Unwrap the wrapper to BE the inner `(type …)` decl (adopt its full child list), so the type
                // declaration takes effect exactly as an un-annotated `(type …)` would — the `@invariant`
                // wrapper is consumed here (recorded above), never surfacing as a top-level annotation form.
                if let Struct::List(inner_children) = ast.get(inner).clone() {
                    ast.structure[i] = Struct::List(inner_children);
                }
                continue;
            }
        }
        // The inner must be a `(def SIG BODY …)` — read its children to adopt them + find the BODY occ.
        // NOTE: this def check is BEFORE the malformed-`@tag` recording below on purpose — the `@tag`
        // contract only applies when the annotation wraps a DEFINITION, so a `@tag` around a NON-def is
        // handled solely by the existing "annotation wraps no definition" rejection; recording a
        // malformed-tag fault here too would double-diagnose one mistake (Copilot PR#484).
        let Some(def_tail) = ast.as_form(inner, "def") else {
            continue; // an annotation around a non-def — leave untouched (a well-formedness concern elsewhere)
        };
        // A `(tag …)` HEAD that is NOT a valid `@tag("string")` — the arg is not exactly one STRING (a
        // number `@tag(5)`, a bare name `@tag(foo)`, zero args `@tag()`, or two `@tag("a" "b")`). This is
        // always an author mistake: silently ignoring it (recording no tag) masks the error, so record the
        // offending `(tag …)` occurrence for `collect_faults` to REJECT (a malformed tag annotation). Only
        // meaningful once the inner IS a def (checked above) — a `@tag` on a non-def is already the
        // "wraps no definition" mistake, not additionally a malformed-tag one.
        if tag_app.is_some() && tag_arg.is_none() {
            malformed_tags.push(name_occ);
        }
        // A `@requires`/`@ensures` with not-exactly-one arg — record for rejection (b4a2), same discipline
        // as the malformed `@tag` above: only once the inner IS a def (a `@requires` on a non-def is the
        // "wraps no definition" mistake). Silently recording no predicate would mask the author error.
        if malformed_verify_here {
            malformed_verify.push(name_occ);
        }
        // The def's BODY occurrence: `def_tail = [SIG, BODY, …]`, so index 1 (a well-formed def has ≥2).
        let Some(&body) = def_tail.get(1) else {
            continue;
        };
        // Rewrite the WRAPPER node to BE the inner def (adopt its full child list `[def-head, SIG, BODY…]`).
        let Struct::List(inner_children) = ast.get(inner).clone() else {
            continue;
        };
        ast.structure[i] = Struct::List(inner_children);
        // The match arms below record exactly the SEMANTIC annotations; `KNOWN_ANNOTATIONS` is the
        // published catalog of those names. Assert the two agree, so adding a name to one without the other
        // trips in debug (a known name that falls to the transparent `_ => {}` would silently lose its
        // policy). An unknown name is intentionally not in the catalog → the `_` arm's inert unwrap.
        debug_assert!(
            name.as_deref()
                .is_none_or(|n| KNOWN_ANNOTATIONS.contains(&n)
                    == matches!(
                        n,
                        "inline-never"
                            | "inline-always"
                            | "test"
                            | "exhaustive"
                            | "requires"
                            | "ensures"
                            // `invariant` is call-style (`(invariant pred)`) so its `name` read is `None` and
                            // it is handled + `continue`d above, never reaching here as a bare name; listed
                            // for catalog parity with `KNOWN_ANNOTATIONS` (like `requires`/`ensures`).
                            | "invariant"
                    )),
            "KNOWN_ANNOTATIONS and the strip_annotations match arms disagree on `{name:?}`"
        );
        match name.as_deref() {
            Some("inline-never") => {
                inline_never.insert(body);
            }
            Some("inline-always") => {
                inline_always.insert(body);
            }
            Some("test") => {
                tests.insert(body);
            }
            // `@exhaustive` marks a property test to drive over its ENTIRE finite domain rather than by
            // random sampling. It also makes the def a test (an `@exhaustive` def need not ALSO be
            // `@test`-marked — recording it in `tests` too means `test_defs` hoists it), so record BOTH.
            Some("exhaustive") => {
                exhaustive.insert(body);
                tests.insert(body);
            }
            // An unknown annotation name (or a non-name annotation head): unwrapped above, recorded in no
            // policy set — a transparent no-op marker so the wrapped def still takes effect. A call-style
            // `@tag("…")` lands here (its name is an application, not a bare name) and is recorded below.
            _ => {}
        }
        // A `@tag("…")` records its string against the def's BODY occ, accumulating across stacked tags
        // (`@tag("slow") @tag("net")` → both). Independent of `@test` — a tag is metadata; the `cdz test
        // --tag` filter reads it via `Db::tags_of`. Recorded AFTER the unwrap so `body` is the def's occ.
        if let Some(tag) = tag_arg {
            tags.entry(body).or_default().push(tag);
        }
        // `@requires(pred)`/`@ensures(pred)` record their predicate occ against the def's BODY occ,
        // accumulating across stacked annotations (a conjunction). The name position is the `(requires …)`
        // / `(ensures …)` application, so `name` (the bare-name read) is `None` and these fall to the `_`
        // arm above — recorded here, like `@tag`, after the unwrap so `body` is the def's occ.
        if let Some(pred) = requires_pred {
            requires.entry(body).or_default().push(pred);
        }
        if let Some(pred) = ensures_pred {
            ensures.entry(body).or_default().push(pred);
        }
    }
    StrippedAnnotations {
        inline_never,
        inline_always,
        tests,
        tags,
        exhaustive,
        malformed_tags,
        requires,
        ensures,
        invariants,
        malformed_verify,
    }
}

/// The one cheap top-level scan: gather the definitions, export requests, and `(type …)` declarations
/// from the top form only, without entering any body. Recognizes `(module NAME item…)`, a bare
/// `(do item…)`, or a lone item.
fn scan_top_level(ast: &Arenas) -> TopScan {
    let mut defs: Vec<Def> = Vec::new();
    let mut exports: Vec<Export> = Vec::new();
    let mut types: Vec<TypeDecl> = Vec::new();
    let mut effects: Vec<EffectDecl> = Vec::new();
    let mut modules: Vec<ModuleDecl> = Vec::new();

    for item in top_items(ast) {
        if let Some(tail) = ast.as_form(item, "def") {
            // Two def SHAPES share the top-level scan (the same two `resolve::do_def_binds` reads inside a
            // `do`): a FUNCTION/nullary def `(def (NAME param…) BODY)` whose signature is a LIST, and a
            // VALUE def `(def NAME VALUE)` whose signature is a bare NAME atom (`(def answer 42)` — the
            // name-plus-value form a module/top-level uses for a constant binding). A value def has NO
            // parameters; its `VALUE` is the body. Distinguished by whether the first element is a list
            // (a signature) or an atom (a value-def name).
            let (name, params) = match tail.first().map(|&s| (s, ast.get(s))) {
                Some((_, Struct::List(children))) if !children.is_empty() => {
                    let name = ast.as_name(children[0]).unwrap_or("").to_string();
                    (name, children[1..].to_vec())
                }
                // `(def NAME VALUE)` — a bare-name VALUE definition (no parameters).
                Some((sig, Struct::Atom(_))) => {
                    (ast.as_name(sig).unwrap_or("").to_string(), Vec::new())
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
                internal: false,
            });
        } else if let Some(tail) = ast.as_form(item, "export") {
            // An `(export a b …)` clause exports EVERY name in its tail — the multi-name surface the ML
            // reader writes `export { a, b, … }` and the printer round-trips (per `is_export_shape`). One
            // `Export` per name, each sharing the clause `occ` but carrying its OWN `name_occ`. A non-name
            // element (a malformed `(export a 5)`) is caught by the well-formedness pass; skip it here so
            // the well-formed names still register (matching how `scan_type_decl`/`scan_effect_decl` scan
            // per element). Reading only `tail.first()` — the prior behavior — SILENTLY dropped every name
            // past the first, so a valid `(export main helper)` published only `main`.
            for &s in tail.iter() {
                // A CONSTRUCTOR-EXPORT element — the list `(. T A)` / `(. T *)` (`as_name` is `None`, so
                // it was already skipped) OR the wildcard ATOM `T.*` (`as_name` is `Some("T.*")`, so it
                // WOULD be pushed as a bare export naming no definition → a misleading CDZ0101). Route both
                // to `ctor_export_elements`' semantic validation, not the bare-export path.
                if is_ctor_export_shape(ast, s) {
                    continue;
                }
                if let Some(name) = ast.as_name(s) {
                    exports.push(Export {
                        name: name.to_string(),
                        def: None,
                        occ: item,
                        name_occ: s,
                    });
                }
            }
        } else if ast.as_form(item, "type").is_some()
            && let Some(decl) = scan_type_decl(ast, item)
        {
            types.push(decl);
        } else if ast.as_form(item, "effect").is_some()
            && let Some(decl) = scan_effect_decl(ast, item)
        {
            effects.push(decl);
        }
    }

    // NESTED type/effect declarations — a `(type …)` / `(effect …)` written inside a `do` block that is
    // NOT a top-level item (`(def (main) (do (type Color …) …))`, a local sum). `top_items` sees only the
    // root's direct children, so descend the def bodies + any nested `do` blocks to gather these too.
    // Their identity is nominal (the declaration occurrence), so a synthesized nested sum resolves through
    // the same occurrence-keyed `type_decls` / `def_by_name` paths as a top-level one — a nested type is
    // brought into scope program-wide (well-formed local declarations do not collide by name, so global
    // visibility is a safe over-approximation of "scoped to its `do`"). A do-local `(def …)` is bound
    // lazily by `resolve`'s do-case; a nested `(type …)` needs the sum RECORD synthesized here, which is
    // why the declaration must be gathered at load rather than resolved on demand.
    let top: std::collections::HashSet<StructId> = top_items(ast).into_iter().collect();
    for &body in defs.iter().filter_map(|d| d.body.as_ref()) {
        collect_nested_decls(ast, body, &top, &mut types, &mut effects, &mut modules);
    }
    // A `(module …)` that is a TOP-LEVEL ITEM — an element of a top-level `(do …)` sequence root, `(do
    // (module m …) (def (main) …) (export main))`. The main scan loop above handles only def/export/type/
    // effect items (no `module` branch), and `collect_nested_decls` SKIPS a `top`-set form (it treats it as
    // "already scanned"), so such a module was registered by NEITHER path — its name stayed unbound and its
    // members escaped type-checking (a bare `(module m …)` and a def-body-nested one both register + check).
    // Register each top-level module item here via the shared `collect_module_decl` (which also descends its
    // members for deeper nesting), matching the do-local / bare-module paths. A module reached as a def
    // body's `(do …)` element is already handled by the `collect_nested_decls` descent above.
    for &item in &top {
        if ast.as_form(item, "module").is_some() {
            collect_module_decl(ast, item, &top, &mut types, &mut effects, &mut modules);
        }
    }

    // A DECLARED type name is NOT an implicit type parameter, even lowercase. `collect_type_params`
    // captures every free lowercase payload name as a tyvar (the `a` in `(type Box (W a))`), following
    // the "types are Capitalized, lowercase is a type variable" convention. But a type is a VALUE, so a
    // declared type name — of ANY case — referenced in a field is a reference to that type, not a fresh
    // variable: `(type mylist (Nil) (Cons Int64 mylist))` self-references `mylist`, and `(type wrap (W
    // num))` over a declared `(type num …)` references `num`. Without this the reference re-lexed as a
    // tyvar, the sum silently became generic, and its variants failed to resolve (a confusing CDZ0203
    // far from the cause). Now that ALL type names are gathered (top-level + nested), drop any param that
    // names a declared type — so a lowercase self/cross type reference resolves to the type (step 3 of
    // `resolve_name`) instead of being captured. A genuine tyvar (`a`, matching no declaration) is kept.
    // (Runs here, after the full gather, because a forward/self reference needs the whole type-name set.)
    let declared_type_names: std::collections::HashSet<String> =
        types.iter().map(|t| t.name.clone()).collect();
    for t in &mut types {
        t.params.retain(|p| !declared_type_names.contains(p));
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

    (defs, exports, types, effects, modules)
}

/// Scan an `(effect NAME (op f (-> A B)) …)` declaration at `item` into an [`EffectDecl`] — the effect
/// name and each operation (its name occurrence + type occurrence). `synth` is left `None` (filled by
/// `effects::synthesize`). Returns `None` if `item` is not an `(effect …)` form. The effect analogue of
/// `scan_type_decl`. A malformed operation clause (not `(op NAME TYPE)`) is skipped; the name may be
/// empty (malformed `(effect)`), in which case it binds nothing.
pub(crate) fn scan_effect_decl(ast: &Arenas, item: StructId) -> Option<EffectDecl> {
    let tail = ast.as_form(item, "effect")?;
    let name = tail
        .first()
        .and_then(|&s| ast.as_name(s))
        .unwrap_or("")
        .to_string();
    let mut ops = Vec::new();
    for &clause in tail.iter().skip(1) {
        // Each operation is `(op NAME TYPE)`. The op keyword names the clause; the first tail element is
        // the operation name, the second its type expression `(-> Param Result)`.
        let Some(op_tail) = ast.as_form(clause, "op") else {
            continue;
        };
        let Some(&name_occ) = op_tail.first() else {
            continue;
        };
        let op_name = ast.as_name(name_occ).unwrap_or("").to_string();
        // The OPTIONAL 3rd op-child is the `@resource`-marker sibling `(resource <idx>)` (v-syntax
        // 2026-08-13): the 0-based param position designated the SEC-F1 resource. Absent for a target-free
        // op. Read the index off the `resource` form's Int atom (rcdzc's `as_int` → `usize`); a malformed
        // sibling (no int) leaves `resource = None`, so a garbled marker degrades to target-free rather
        // than mis-indexing.
        let resource = op_tail
            .get(2)
            .and_then(|&s| ast.as_form(s, "resource"))
            .and_then(|res_tail| res_tail.first().copied())
            .and_then(|idx_occ| ast.as_int(idx_occ))
            .and_then(|iv| iv.to_i64())
            .and_then(|n| usize::try_from(n).ok());
        ops.push(OpDecl {
            name: op_name,
            name_occ,
            ty: op_tail.get(1).copied(),
            resource,
        });
    }
    Some(EffectDecl {
        name,
        occ: item,
        ops,
        synth: None,
    })
}

/// Scan a `(type NAME variant…)` declaration at `item` into a [`TypeDecl`] — the name, each variant
/// (its name occurrence + payload type occurrences), and the implicit type parameters (free lowercase
/// payload names, first-appearance order). `synth` is left `None` (filled by `sums::synthesize`).
/// Returns `None` if `item` is not a `(type …)` form. Shared by the top-level scan and the built-in
/// prelude-sum synthesis, so a prelude `Option`/`Result` declaration scans exactly like a user one.
pub(crate) fn scan_type_decl(ast: &Arenas, item: StructId) -> Option<TypeDecl> {
    let tail = ast.as_form(item, "type")?;
    // The type NAME is the first tail element in TWO spellings: a BARE atom `(type Box (Mk a))` — the name
    // is the atom — OR a PARENTHESIZED head `(type (Box a b…) …)` — a `(Name params…)` list whose HEAD atom
    // is the name and whose tail are the declared type parameters. Both are canonical in the corpus (`(type
    // Box (Box a))` and `(type (Box a) (Full a) (Nil unit))`). Without the list-head case, `as_name` on the
    // `(Box a)` list returned `None` → the name defaulted to `""`, so the type was registered under the
    // empty string: it worked in VALUE position (resolved by its VARIANT names) but was UNRESOLVABLE by
    // NAME in a TYPE-EXPRESSION position — `(: b (Box Int64))` / `(: b Box)` / a `(Wrap (Box Int64))`
    // payload reported CDZ0101 "unknown type `Box`", while the built-in `(Option Int64)` (prelude) worked.
    let head = tail.first().copied();
    // The type NAME via the shared decoder (bare atom `(type Box …)` OR parenthesized `(type (Box a) …)`
    // head), so every raw `(type …)`-tail name-reader agrees (see `Arenas::type_decl_head_name`).
    let name = head
        .and_then(|s| ast.type_decl_head_name(s))
        .unwrap_or("")
        .to_string();
    // Explicit HEAD params from a parenthesized `(Name params…)` head — the lowercase tail atoms, in
    // first-appearance order, DE-DUPED (a `(type (Box a a) …)` names `a` once — mirrors `collect_type_params`
    // below, so `decl.params.len()` is the true arity, not an overcount). `unit` is the value/type, never a
    // param. Empty for a bare-atom head. Payload-implied params are appended (also de-duped) below.
    let head_params: Vec<String> = match head.map(|s| ast.get(s)) {
        Some(Struct::List(kids)) => {
            let mut ps: Vec<String> = Vec::new();
            for &p in kids.iter().skip(1) {
                if let Some(n) = ast.as_name(p)
                    && n.starts_with(|c: char| c.is_ascii_lowercase())
                    && n != "unit"
                    && !ps.iter().any(|q| q == n)
                {
                    ps.push(n.to_string());
                }
            }
            ps
        }
        _ => Vec::new(),
    };
    // An OPEN sum ends in a trailing `.. r` row-variable marker (`type-system.md §204/§208`): `(type T
    // (Known Int64) .. r)`. The `..` token already lexes (list-rest); here it marks the sum OPEN and `r`
    // (a lowercase name) is the row variable. Detect it as the two FINAL elements of the `(type …)` tail
    // and STRIP them so they are not mistaken for two nullary variants. Closed stays the default — a tail
    // with no trailing `.. name` yields `open_tail: None` (every existing corpus sum is unchanged). The
    // row variable's name is NOT registered as a type parameter (it stands for the open variant set, not a
    // payload-position generic), so `collect_type_params` below never sees it (it scans only payloads).
    let mut items: Vec<StructId> = tail.iter().skip(1).copied().collect();
    let open_tail = {
        let n = items.len();
        // An open row ends in a rest naming a lowercase ROW VARIABLE — the flat `.. rowvar` (two trailing
        // items) or the wrapped `(.. rowvar)` node (one). `rest_marker` recognizes both; `trailing_start ==
        // n` requires it to be the FINAL element (a row var binds the open tail), and `truncate(k)` drops
        // from the marker index (flat: `..`+rowvar = 2; wrapped: the `(.. rowvar)` node = 1).
        if let Some((k, rowvar_occ, trailing_start)) = ast.rest_marker(&items)
            && trailing_start == n
            && ast
                .as_name(rowvar_occ)
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_lowercase()))
        {
            let rowvar = ast.as_name(rowvar_occ).unwrap().to_string();
            items.truncate(k);
            Some(rowvar)
        } else {
            None
        }
    };
    let mut variants = Vec::new();
    for &v in &items {
        // A leading `(doc "…")` metadata form (the ML reader attaches a `///` doc comment on a type as a
        // `(doc …)` form after the type NAME — `parser.rs` `type_expr`) is NOT a variant: skip it, exactly
        // as `strip_def_docs` drops a leading doc on a `(def …)`. Without this, `///`-documented type
        // declarations mis-parse the doc as a bogus `doc` variant (CDZ0201 "declared more than once").
        if ast.as_form(v, "doc").is_some() {
            continue;
        }
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
                ctor: None,
            });
            // (`ctor` is filled by `sums::synthesize` once the record is built.)
        }
    }
    // Collect the type parameters. EXPLICIT head params (`(type (Box a) …)` → `["a"]`) come first, in the
    // order declared; then the IMPLICIT payload params — a free LOWERCASE name in any variant payload, in
    // first-appearance order (`(type Option (Some a) None)` → `["a"]`; a Capitalized name is a type). A
    // param already named in the head is not re-added (a head-declared `a` mentioned again in a payload is
    // one param). This keeps a HEAD-ONLY (phantom) param and de-dups the common case where the head param
    // also appears in a payload.
    let mut params: Vec<String> = head_params;
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
        open_tail,
        synth: None,
        // A scanned decl (user OR prelude) declares no associated members here; the built-in `Ast`'s are
        // attached in `sums::prelude_decls` (prelude-defined), consumed generically by `sum_record`.
        associated: Vec::new(),
    })
}

/// Collect the IMPLICIT type parameters mentioned in a payload type expression at `occ`, appending each
/// new one to `params` in first-appearance order. A parameter is a free LOWERCASE name (types are
/// Capitalized — `Int64`, `Bool`, `Option` — so a lowercase name in type position is a type variable,
/// the same convention the prelude's operator type-lambdas use with `a`). Descends a type application
/// `(Option a)` / `(Tuple a b)` into its arguments. A duplicate is not re-added (a `HashSet`-like
/// linear check keeps the small param list ordered by first appearance).
///
/// WARNING: A `(Record (field Type)…)` payload's field NAME is a LABEL, not a type expression — a lowercase
/// field name (`(Record (v Int64))`) must NOT be mistaken for a type parameter, or the sum spuriously
/// becomes generic over `v` and its ctor arrow breaks (the payload reads as an unresolvable variable, so
/// the variant looks nullary). So a `(Record …)` form descends only into each field pair's TYPE (the
/// second element), skipping the name — the same asymmetry `reduce_ctor`/`decode_ty` apply to a record
/// field pair. Every OTHER form (`Tuple`/`List`/`Option`/`->`) descends all children uniformly.
fn collect_type_params(ast: &Arenas, occ: StructId, params: &mut Vec<String>) {
    match ast.get(occ) {
        Struct::Atom(_) => {
            if let Some(n) = ast.as_name(occ)
                && n.starts_with(|c: char| c.is_ascii_lowercase())
                // `unit` is the prelude UNIT value/type (the empty product), NOT a type PARAMETER. Without
                // this, a variant payload `(None unit)` / `(Nil unit)` harvested `unit` as a spurious type
                // param, so `(type (Box a) (Full a) (Nil unit))` read as 2-parameter `[a, unit]` → `(Full 1)`
                // typed `Sum{Box, args:[Int64, <free Var>]}` (the unfilled phantom); the stray free Var left
                // the sum non-Eq/non-Ord → a Set/Map of it DECLINED on the rust backend (wasm erased the open
                // arg + tolerated it). `unit` in a type position instead reduces to `Ty::Unit` (a concrete
                // type — see `typeval_of`'s unit arm), so the pervasive nullary-variant idiom `(None unit)`
                // (prelude.rs §"the pervasive nullary-variant idiom"; guide PatternMatching) keeps resolving
                // unchanged — it is NOT a param. The bare-atom analogue of the lowercase compound-type
                // ALIASES (`tuple`/`record`/`list`/`map`) skipped in the List arms below.
                && n != "unit"
                && !params.iter().any(|p| p == n)
            {
                params.push(n.to_string());
            }
        }
        // A `(Record (field Type)…)` type: a field NAME is a label, never a type parameter. Descend only
        // into each field pair's TYPE element, skipping the name (which may be lowercase — `(Record (v
        // Int64))` — and would otherwise be collected as a spurious param). Matches BOTH the capital
        // `Record` and the lowercase `record` ALIAS — the lowercase compound-type aliases (`tuple`/
        // `record`/`list`/`map`) are recognized type constructors in a payload-type position exactly as
        // they are in an annotation position (`(: e (record …))`), so their head is a TYPE CTOR, not a
        // type parameter. Without matching `record` here, a `(type P (Mk (record (x Int64))))` harvested
        // the lowercase head `record` (and each field label) as spurious params, making `P` bogus-generic
        // → the `Mk` ctor scheme mis-reduced and read as NULLARY (CDZ0201/0203 on construction).
        Struct::List(children)
            if matches!(
                children.first().and_then(|&h| ast.as_name(h)),
                Some("Record") | Some("record")
            ) =>
        {
            for &pair in children.iter().skip(1) {
                if let Struct::List(items) = ast.get(pair) {
                    // The field TYPE occurrence — the name is a label, skipped. A field is either a
                    // 2-element `(name Type)` pair (s-expr `(Record (v a))`) OR a 3-element `(: name Type)`
                    // annotation triple (the ML surface `{v: a}` lowering). Without the triple case, a
                    // param mentioned in an ML-surfaced record field (`(type Box (Box (Record (: v a))))`)
                    // was NOT collected → `decl.params` empty → no ctor type-lambda → the `a` in the ctor
                    // arrow stayed free → `typeval_of` gave None → the ctor read NULLARY (CDZ0201 at
                    // construction). The `collect_type_params` companion of the `(: name type)` decode the
                    // RecordCtor reducers already accept.
                    match items.as_slice() {
                        [_name, ty] => collect_type_params(ast, *ty, params),
                        [colon, _name, ty] if ast.as_name(*colon) == Some(":") => {
                            collect_type_params(ast, *ty, params);
                        }
                        _ => {}
                    }
                }
            }
        }
        // A lowercase compound-type ALIAS application whose args are all TYPES — `(tuple T…)`, `(list T)`,
        // `(map K V)`. The head (`tuple`/`list`/`map`) is a recognized type constructor (a prelude alias),
        // NOT a type parameter, so it must be SKIPPED — the capital spellings (`Tuple`/`List`/`Map`) are
        // already skipped by the lowercase filter in the `Atom` arm, but the lowercase alias head would
        // otherwise be harvested. Descend into the type arguments only (they may hold a real param, e.g.
        // `(tuple a Int64)`). Without this a `(type P (Mk (tuple Int64 Int64)))` harvested `tuple` as a
        // spurious param → `P` bogus-generic → `Mk` read as NULLARY (CDZ0201 on construction). (`record`
        // is handled above with its label-skipping; a lowercase `set` alias does not exist in the reader.)
        Struct::List(children)
            if matches!(
                children.first().and_then(|&h| ast.as_name(h)),
                Some("tuple") | Some("list") | Some("map")
            ) =>
        {
            for &arg in children.iter().skip(1) {
                collect_type_params(ast, arg, params);
            }
        }
        // A `(Qty T u)` quantity type: the FIRST argument is the inner numeric TYPE, but the SECOND is a
        // compile-time UNIT expression (`(Unit.base #"meter")`, `(Unit.* …)`) whose leaf names are UNIT
        // BASES, not type variables — `eval::QtyCtor` reads it via `unit_of` and `resolve::decode_ty`'s
        // "Qty" arm decodes it as a `Unit`, never as a type. Descending into it uniformly would harvest a
        // lowercase unit-builder name (`base` in `Unit.base`) as a spurious type parameter, so a
        // `(type Holder (H (Qty Rational (Unit.base #"meter"))))` becomes generic `Holder(base)` and a
        // bare `Holder` stops resolving (CDZ0203). Descend ONLY into the inner-type argument; skip the
        // unit. Same asymmetry `decode_ty`/`push_payload_type_positions` apply. (A malformed-arity `Qty`
        // falls through to the uniform descent below — a well-formedness fault reported elsewhere.)
        Struct::List(children)
            if children.first().and_then(|&h| ast.as_name(h)) == Some("Qty")
                && children.len() == 3 =>
        {
            collect_type_params(ast, children[1], params);
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
/// Gather `(type …)` / `(effect …)` declarations nested inside a `do` block reachable from `body` —
/// the local-declaration companion of `scan_top_level`'s top-item loop. A declaration only appears as a
/// FORM of a `do` block, so descend `do` blocks (and the bodies of any do-local `(def …)`), collecting
/// each `(type …)`/`(effect …)` form. `top` is the set of top-level items already scanned (skip them so a
/// top-level type in a `(do …)` root is not counted twice). Bounded to declaration-bearing structure
/// (`do` forms + def bodies), not arbitrary expressions, so a future quoted `(type …)` datum is not
/// mistaken for a declaration.
fn collect_nested_decls(
    ast: &Arenas,
    body: StructId,
    top: &std::collections::HashSet<StructId>,
    types: &mut Vec<TypeDecl>,
    effects: &mut Vec<EffectDecl>,
    modules: &mut Vec<ModuleDecl>,
) {
    let Some(forms) = ast.as_form(body, "do") else {
        return;
    };
    for &form in forms {
        if top.contains(&form) {
            continue; // a top-level item (the root `do`'s own child) — already scanned
        }
        if ast.as_form(form, "type").is_some() {
            if let Some(decl) = scan_type_decl(ast, form) {
                types.push(decl);
            }
        } else if ast.as_form(form, "effect").is_some() {
            if let Some(decl) = scan_effect_decl(ast, form) {
                effects.push(decl);
            }
        } else if ast.as_form(form, "module").is_some() {
            // A do-local `(module NAME member…)` — register it and descend its members (including any
            // NESTED `(module …)`) via the shared `collect_module_decl`, which handles arbitrarily deep
            // module nesting.
            collect_module_decl(ast, form, top, types, effects, modules);
        } else if let Some(def_tail) = ast.as_form(form, "def") {
            // A do-local `(def sig body)` whose body may itself be a `(do …)` carrying declarations.
            if let Some(&def_body) = def_tail.get(1) {
                collect_nested_decls(ast, def_body, top, types, effects, modules);
            }
        } else {
            // Any other form may itself be (or contain) a nested `(do …)` — descend it directly.
            collect_nested_decls(ast, form, top, types, effects, modules);
        }
    }
}

/// Register a single `(module NAME member…)` DECLARATION and descend its members for further nested
/// declarations — the recursive core shared by `collect_nested_decls`'s do-local module branch and its
/// own recursion for a MODULE-IN-MODULE member. A `(def …)` member becomes an export FIELD; a nested
/// `(module inner …)` member is itself registered (so `inner` is a field of the outer's record — a
/// nested record) and recursed into, for arbitrarily deep module nesting. An `(effect …)`/`(op …)`/
/// `(type …)`/`(doc …)` member is a legitimate NON-export (correctly ABSENT from the record, so
/// projecting it is the closed-record CDZ0201 the corpus wants). A member the compiler does NOT model as
/// either a field or a benign non-export — a `(pragma …)` (a validation OBLIGATION not yet built) —
/// blocks registration: the module NAME stays unbound and the program DECLINES rather than silently
/// dropping the obligation (decline-don't-miscompile). The modeled set is closed; anything outside it
/// (today just `pragma`) blocks. The `all_modeled` guard is per-module, so an unmodeled member in the
/// INNER module blocks only the inner's registration, not the outer's — matching a top-level module's
/// independence.
fn collect_module_decl(
    ast: &Arenas,
    form: StructId,
    top: &std::collections::HashSet<StructId>,
    types: &mut Vec<TypeDecl>,
    effects: &mut Vec<EffectDecl>,
    modules: &mut Vec<ModuleDecl>,
) {
    let Some(mod_tail) = ast.as_form(form, "module") else {
        return;
    };
    let members = mod_tail.get(1..).unwrap_or(&[]).to_vec();
    // A `(pragma default-integer <T>)` member is MODELED — its VALIDATION is built (CDZ0601/0602/0303,
    // `compile::collect_faults`) and its EFFECT (a bare literal in this module defaults to `<T>`) is
    // realized via `ModuleDecl::default_int` + the load-time literal map, so it is not an unmodeled
    // obligation blocking registration. A `(pragma …)` with any OTHER key is still unmodeled (blocks) —
    // the modeled set is exactly the keys whose meaning the compiler realizes.
    // A `(pragma <key> …)` member is MODELED for a key whose EFFECT the compiler realizes: `default-integer`
    // (a bare literal defaults to `<T>`), `default-fraction` (a bare numeric literal grounds to the exact
    // rational `<T>`), and `default-float` (a bare decimal literal defaults to the float type `<T>`). Each
    // is realized via a `ModuleDecl` field + a load-time literal map, so they don't block registration; any
    // OTHER pragma key stays unmodeled (blocks).
    let is_modeled_pragma = |member: StructId| {
        ast.as_form(member, "pragma").is_some_and(|t| {
            matches!(
                t.first().and_then(|&k| ast.as_name(k)),
                Some("default-integer" | "default-fraction" | "default-float" | "overflow")
            )
        })
    };
    let modeled = |member: StructId| {
        matches!(
            ast.head_name(member),
            // `export` is modeled: a `(module m … (export a b))` member NAMES the module's visible fields
            // (`modules-and-namespaces.md` §Visibility Is Explicit; the ML surface `export { a, b }` emits
            // it). `modules::module_record` reads it to filter the record; here it just must not block
            // registration. Its VALIDATION (a duplicate export, an export of an undefined name) rides the
            // ordinary well-formedness pass, exactly as a top-level `(export …)`'s does.
            Some("def" | "effect" | "op" | "type" | "module" | "doc" | "export")
        ) || is_modeled_pragma(member)
    };
    let all_modeled = members.iter().all(|&m| modeled(m));
    // The type-expression (2nd operand) of a well-formed `(pragma <key> <T>)` member for `key`. A malformed
    // pragma (wrong arity) is caught by the CDZ0602 validation; here take the type occ when present.
    let pragma_ty = |key: &str| {
        members.iter().find_map(|&m| {
            let t = ast.as_form(m, "pragma")?;
            (t.first().and_then(|&k| ast.as_name(k)) == Some(key))
                .then(|| t.get(1).copied())
                .flatten()
        })
    };
    let default_int = pragma_ty("default-integer");
    let default_fraction = pragma_ty("default-fraction");
    let default_float = pragma_ty("default-float");
    // The overflow policy — parsed from the whole `(pragma overflow (signed …) (unsigned …))` member (its
    // operands are sub-forms, not a single type-expr, so `pragma_ty` does not apply).
    let overflow = members
        .iter()
        .find(|&&m| {
            ast.as_form(m, "pragma")
                .and_then(|t| t.first().and_then(|&k| ast.as_name(k)))
                == Some("overflow")
        })
        .and_then(|&m| parse_overflow_spec(ast, m));
    if all_modeled
        && let Some(&name) = mod_tail.first()
        && let Some(name_str) = ast.as_name(name)
    {
        modules.push(ModuleDecl {
            name: name_str.to_string(),
            occ: form,
            synth: None,
            default_int,
            default_fraction,
            default_float,
            overflow,
        });
    }
    for &member in &members {
        if ast.as_form(member, "module").is_some() {
            // A MODULE-IN-MODULE member — register + descend it (a nested record field of this module).
            collect_module_decl(ast, member, top, types, effects, modules);
        } else if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            // A `(def …)` member whose body may itself carry a `(do …)` with declarations.
            collect_nested_decls(ast, def_body, top, types, effects, modules);
        }
    }
}

/// Record every bare integer-LITERAL node in the DEF-BODY subtrees of a `(pragma default-integer <T>)`
/// module `mod_form` → the pragma's `<T>` occurrence, into `out`. DEFINITION-SITE scoped: it descends
/// only the module's OWN `(def …)` member bodies (a literal in a NESTED `(module inner …)` member is
/// governed by `inner`'s OWN pragma, not this one — so nested modules are NOT descended here; each is
/// walked when `collect_default_int_literals` is called for it). Keyed by the ORIGINAL literal node, so a
/// later β-copy that reparents the literal (moving it out of the module structurally) still finds its
/// default via this map — the parent-walk a naive lookup would use is unusable post-copy.
///
/// `(pragma default-integer <T>)` is a MEANING-CHANGING directive — it changes the type its module's bare
/// literals take — and it is read HERE from the module's CANONICAL AST (`mod_form`, the pragma node in the
/// binary AST), never from a compilation option outside the form. So a module's meaning is determined by
/// its canonical form alone.
//= spec/capabilities/modules-and-namespaces.md#a-meaning-changing-directive-is-part-of-the-canonical-form
//# A module directive that changes the meaning of the module's definitions MUST be carried in the module's canonical form, so that the module's meaning is determined by its canonical form alone and does not depend on a compilation option outside it.
fn collect_default_int_literals(
    ast: &Arenas,
    mod_form: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        // Only a `(def …)` member's BODY carries value literals to default; a nested `(module …)` has its
        // own scope (skip), and `pragma`/`type`/`effect`/`doc` members carry no value literals.
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_int_literals(ast, def_body, ty_expr, out);
        }
    }
}

/// Mark `node` and every descendant into `out` — the subtree marker behind [`Db::type_expr_nodes`]. A
/// type expression `(List Ast)` / `(-> A B)` and everything inside it is a type, never a value construct,
/// so the whole subtree is off-limits to the construct-position variant shadow. Bounded by the subtree
/// size; a node visited twice (overlapping roots cannot occur, but defensively) is idempotent.
fn mark_subtree(ast: &Arenas, node: StructId, out: &mut crate::fxhash::FxHashSet<StructId>) {
    if !out.insert(node) {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_subtree(ast, c, out);
        }
    }
}

/// Mark every integer-literal node reachable from `node` (recursively) with `ty_expr` in `out`, WITHOUT
/// descending into a nested `(module …)` (its own scope). A literal already recorded is left as-is (the
/// nearest enclosing pragma wins — the outer walk visits an inner module separately with its own type).
fn mark_int_literals(
    ast: &Arenas,
    node: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    if ast.as_int(node).is_some() {
        out.entry(node).or_insert(ty_expr);
        return;
    }
    // A nested module carries its OWN default (or none) — do not leak this module's default into it.
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_int_literals(ast, c, ty_expr, out);
        }
    }
}

/// The `trap`/`wrap` mode named by a `(signed <mode>)` / `(unsigned <mode>)` sub-form's argument `node`, or
/// `None` if it is not one of the two mode names (a malformed mode is reported as CDZ0602 by the pragma
/// validation pass; here an unrecognized name simply yields no policy for that signedness).
fn parse_overflow_mode(ast: &Arenas, node: StructId) -> Option<OverflowMode> {
    match ast.as_name(node)? {
        "trap" => Some(OverflowMode::Trap),
        "wrap" => Some(OverflowMode::Wrap),
        _ => None,
    }
}

/// Parse a `(pragma overflow (signed <mode>) (unsigned <mode>))` form into an [`OverflowSpec`]. Either
/// sub-form may be absent (that signedness stays `None` → falls through to the next precedence level). Order
/// is irrelevant; each sub-form is matched by its head (`signed`/`unsigned`). Returns `None` only when the
/// form carries NEITHER a well-formed `signed` nor `unsigned` sub-form (nothing to record).
fn parse_overflow_spec(ast: &Arenas, pragma_form: StructId) -> Option<OverflowSpec> {
    let ptail = ast.as_form(pragma_form, "pragma")?;
    // Skip the `overflow` key; the rest are the `(signed …)` / `(unsigned …)` sub-forms.
    let mut spec = OverflowSpec::default();
    for &sub in ptail.get(1..).unwrap_or(&[]) {
        if let Some(t) = ast.as_form(sub, "signed") {
            spec.signed = t.first().and_then(|&m| parse_overflow_mode(ast, m));
        } else if let Some(t) = ast.as_form(sub, "unsigned") {
            spec.unsigned = t.first().and_then(|&m| parse_overflow_mode(ast, m));
        }
    }
    (spec.signed.is_some() || spec.unsigned.is_some()).then_some(spec)
}

/// Record every unqualified `+`/`-`/`*` arithmetic node in the DEF-BODY subtrees of a `(pragma overflow …)`
/// module `mod_form` → the module's `spec`, into `out`. The overflow twin of [`collect_default_int_literals`]
/// — DEFINITION-SITE scoped (descends only the module's OWN `(def …)` bodies; a nested `(module inner …)`
/// keeps its own policy), keyed by the ORIGINAL operator node so a later β-copy that reparents the op still
/// finds its policy.
fn collect_overflow_modes(
    ast: &Arenas,
    mod_form: StructId,
    spec: OverflowSpec,
    out: &mut crate::fxhash::FxHashMap<StructId, OverflowSpec>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_overflow_nodes(ast, def_body, spec, out);
        }
    }
}

/// Mark every unqualified `+`/`-`/`*` arithmetic node reachable from `node` (recursively) with `spec` in
/// `out`, WITHOUT descending into a nested `(module …)` (its own scope). A node already recorded is left
/// as-is (the nearest enclosing pragma wins — the outer walk visits an inner module separately). Only the
/// bare `+`/`-`/`*` head is matched: a named `Int64.wrapping-add` / `(. Int64 checked-add)` form is a
/// different head and is IMMUNE (it carries its own overflow contract).
fn mark_overflow_nodes(
    ast: &Arenas,
    node: StructId,
    spec: OverflowSpec,
    out: &mut crate::fxhash::FxHashMap<StructId, OverflowSpec>,
) {
    // A nested module carries its OWN policy (or none) — do not leak this module's into it.
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        // A `(+ a b)` / `(- a b)` / `(* a b)` whose HEAD is the bare operator name is a governed arithmetic
        // op. (`as_name` on the head is `None` for a dotted `(. Int64 wrapping-add)` head — never matched.)
        if let Some(&head) = children.first()
            && matches!(ast.as_name(head), Some("+" | "-" | "*"))
        {
            out.entry(node).or_insert(spec);
        }
        for &c in children {
            mark_overflow_nodes(ast, c, spec, out);
        }
    }
}

/// Mark each bare, un-suffixed integer literal written as the DIRECT argument of a constructor whose
/// corresponding DECLARED payload type is `BigInt`, so `infer` grounds it to `Ty::BigInt` instead of the
/// Int64 default (the operator-approved contextual-grounding: an integer literal grounds to BigInt
/// losslessly, and an explicit context takes precedence over the declared default — a grounding, not a
/// promotion). Keyed by the literal's ORIGINAL occurrence (β-copy-robust, like the default-literal maps).
///
/// SUFFIX DISCIPLINE FALLS OUT OF THE AST SHAPE: a suffixed `42N` reader-desugars to `(: 42 BigInt)` (an
/// annotation node), and a computed/already-typed argument is a `List`/`Annot` node — NONE is a bare
/// `Leaf::Int`. So matching a DIRECT integer-literal argument marks EXACTLY the bare, un-suffixed,
/// uncomputed case; a `42N`, a `(: 42 Int64)`, or a `(+ 1 1)` in a BigInt payload position is never marked
/// and still declines if it mismatches (the operator's load-bearing guard, enforced structurally).
fn collect_bigint_ctor_arg_literals(
    ast: &Arenas,
    type_decls: &[TypeDecl],
    out: &mut crate::fxhash::FxHashSet<StructId>,
) {
    // A constructor NAME → its declared payload type-expression occurrences (declaration order). A variant
    // name is the map key; a dotted head `(. Sum Variant)` matches on its `Variant` segment, a bare head on
    // the name directly. FIRST-wins on a name shared across sums (matches the resolver's own first-wins);
    // an ambiguous same-named ctor with a DIFFERENT payload type is rare and, if wrong, only misses/adds a
    // grounding the type-checker still validates (a mismatch still declines) — never unsound.
    let mut ctor_payloads: crate::fxhash::FxHashMap<&str, &[StructId]> =
        crate::fxhash::FxHashMap::default();
    for decl in type_decls {
        for v in &decl.variants {
            if !v.payloads.is_empty() {
                ctor_payloads.entry(&v.name).or_insert(&v.payloads);
            }
        }
    }
    if ctor_payloads.is_empty() {
        return; // no payload-carrying constructors → nothing to mark
    }
    // Walk EVERY form in the arena; a `(head arg…)` whose head names a known constructor marks each bare
    // integer-literal arg whose matching declared payload type is `BigInt`.
    for ix in 0..ast.structure.len() {
        let id = <StructId as crate::arena::Index>::from_ix(ix);
        let Struct::List(children) = ast.get(id) else {
            continue;
        };
        let Some((&head, args)) = children.split_first() else {
            continue;
        };
        // The constructor's variant name: a bare head `(W 42)` reads via `as_name`; a dotted head
        // `(Ast.Int 42)` is `(. Ast Int)` — take the KEY segment (2nd element of the `.` form).
        let ctor_name = ast.as_name(head).or_else(|| {
            ast.as_form(head, ".")
                .and_then(|t| t.get(1).copied())
                .and_then(|k| ast.as_name(k))
        });
        let Some(name) = ctor_name else { continue };
        let Some(payloads) = ctor_payloads.get(name) else {
            continue;
        };
        for (arg, &payload_ty) in args.iter().zip(payloads.iter()) {
            // ONLY a DIRECT bare integer literal (not an annotation, not a computed expression) whose
            // declared payload type is the bare name `BigInt`.
            if ast.as_int(*arg).is_some() && ast.as_name(payload_ty) == Some("BigInt") {
                out.insert(*arg);
            }
        }
    }
    // SLICE 2 — ANNOTATED COLLECTION ELEMENTS: `(: (list 1 2 3) (List BigInt))` grounds each bare element
    // literal to BigInt (the collection-element analogue of the ctor-payload case — v-metaprogramming's
    // hand-written `(Ast.Int N)` inside a `(list …)`). Walk every annotation `(: <value> <ty-expr>)` and
    // mark the bare integer literals the type-expr expects to be `BigInt`.
    for ix in 0..ast.structure.len() {
        let id = <StructId as crate::arena::Index>::from_ix(ix);
        if let Some(tail) = ast.as_form(id, ":")
            && let [value, ty_expr] = tail
        {
            mark_bigint_expected_literals(ast, *value, *ty_expr, out);
        }
    }
}

/// Mark each bare integer literal in `value` whose expected type (from the annotation type-expr `ty_expr`)
/// is `BigInt`. Descends the two positions where an integer literal sits under a known type-expr shape:
/// a direct `(: 42 BigInt)` (redundant with the annotation path, but harmless), and a
/// `(: (list …) (List BigInt))` whose ELEMENT type is BigInt — each bare-literal element grounds. Only a
/// DIRECT bare `Leaf::Int` is marked (a suffixed/computed/nested-annotated element is a different node and
/// keeps its own type — the same bare-only discipline the ctor-payload walk uses).
fn mark_bigint_expected_literals(
    ast: &Arenas,
    value: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashSet<StructId>,
) {
    // A bare `BigInt` type-expr over a bare integer literal → ground it.
    if ast.as_name(ty_expr) == Some("BigInt") {
        if ast.as_int(value).is_some() {
            out.insert(value);
        }
        return;
    }
    // A `(List BigInt)` type-expr over a list literal: ground each bare-literal element. The TYPE ctor head
    // is always the NAME `List` — the reader emits a type constructor as a name-head application `(List …)`,
    // NEVER a string-head `("List" …)` (that has no reader/ML surface; `from_spelling` is lowercase-VALUE
    // only), so `as_form` (name-head) is the sole live reader-produced arm (verified with v-syntax, M3). The
    // VALUE side (the annotated list literal) is read via `compound_form_of` below, which accepts both the
    // native `#list(…)` ctor-leaf head and the `(list …)`/`("list" …)` aliases.
    if let Some(list_ty_tail) = ast.as_form(ty_expr, "List")
        && let Some(&elem_ty) = list_ty_tail.first()
        && ast.as_name(elem_ty) == Some("BigInt")
        && let Some(elems) = ast.compound_form_of(value, CompoundCtor::List)
    {
        for &e in elems {
            if ast.as_int(e).is_some() {
                out.insert(e);
            }
        }
    }
}

/// The `default-fraction` analogue of [`collect_default_int_literals`]: record every bare NUMERIC literal
/// (integer OR decimal) in the DEF-BODY subtrees of a `(pragma default-fraction <T>)` module → the
/// pragma's `<T>` occurrence. DEFINITION-SITE scoped (own `(def …)` bodies only; a nested module has its
/// own scope). Keyed by the ORIGINAL literal node (β-copy-robust).
fn collect_default_fraction_literals(
    ast: &Arenas,
    mod_form: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_numeric_literals(ast, def_body, ty_expr, out);
        }
    }
}

/// Mark every NUMERIC-literal node (integer via `as_int` OR decimal via `as_float`) reachable from `node`
/// (recursively) with `ty_expr`, WITHOUT descending into a nested `(module …)`. The fraction analogue of
/// [`mark_int_literals`] — an exact default applies to a decimal `0.5` (grounding to `1/2`) as well as an
/// integer, so both leaf kinds are marked. A literal already recorded is left as-is.
fn mark_numeric_literals(
    ast: &Arenas,
    node: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    if ast.as_int(node).is_some() || ast.as_float(node).is_some() {
        out.entry(node).or_insert(ty_expr);
        return;
    }
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_numeric_literals(ast, c, ty_expr, out);
        }
    }
}

/// The `default-float` analogue of [`collect_default_int_literals`]: record every bare DECIMAL literal in
/// the DEF-BODY subtrees of a `(pragma default-float <T>)` module → the pragma's `<T>` occurrence.
/// DEFINITION-SITE scoped (own `(def …)` bodies only; a nested module has its own scope). Keyed by the
/// ORIGINAL literal node (β-copy-robust). Like the integer twin, this is a MEANING-CHANGING directive read
/// from the module's CANONICAL AST alone, so a module's meaning does not depend on a compilation option.
fn collect_default_float_literals(
    ast: &Arenas,
    mod_form: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_float_literals(ast, def_body, ty_expr, out);
        }
    }
}

/// Mark every DECIMAL-literal node (via `as_float`) reachable from `node` (recursively) with `ty_expr`,
/// WITHOUT descending into a nested `(module …)`. The float analogue of [`mark_int_literals`] — a default
/// float width governs how a WRITTEN-DECIMAL literal grounds (`3.14` → `Float32`), not how an integer
/// literal does (an integer keeps its integer default), so ONLY float leaves are marked. A literal already
/// recorded is left as-is.
fn mark_float_literals(
    ast: &Arenas,
    node: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    if ast.as_float(node).is_some() {
        out.entry(node).or_insert(ty_expr);
        return;
    }
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_float_literals(ast, c, ty_expr, out);
        }
    }
}

/// Scan EVERY `(Unit.define #"name" base-unit-expr num den)` form in the arena — a TOP-LEVEL declaration
/// OR an INLINE one in a `Qty.of` unit position — into `(name, base-unit-occurrence, scale-num, scale-den)`
/// entries, the user family-declaration surface. (Walking the whole arena, not just top-level items, is
/// what feeds an inline define the SAME name→conversion uniqueness table `check_unit_defines` consults, so
/// an inline redeclaration of a built-in ratio is rejected CDZ0502 like the top-level form — see the body.)
/// `Unit.define` reads as the member-access head `(. Unit define)` applied to four args: a symbol name,
/// a base-unit expression (reduced later by `eval::unit_of`), and an integer `num`/`den` scaling the
/// base. A malformed form (wrong arity, non-symbol name, non-integer scale) is skipped here — it
/// surfaces as an ordinary fault when the node is checked, not silently. The name's TEXT comes from the
/// `Leaf::Sym`; a non-symbol first arg is not a unit definition.
fn scan_unit_defines(ast: &Arenas) -> Vec<(String, StructId, i128, i128)> {
    let mut out = Vec::new();
    // Scan EVERY arena node, not just top-level items: a `(Unit.define #"name" base num den)` is a
    // name→conversion declaration wherever it appears — a TOP-LEVEL statement OR an INLINE unit value in a
    // `Qty.of` unit position (`(Qty.of a (Unit.define #"foot" (Unit.base #"meter") 2 1))`). Both must feed
    // the SAME uniqueness table `check_unit_defines` consults: else an inline define silently redefines a
    // built-in unit's ratio (foot=2m) or uses one name at two ratios in one expression — the exact
    // silent-wrong-physics the CDZ0502 "A Named Unit's Conversion Is Unique" rule exists to prevent
    // (`units-of-measure.md`). Arena index order is deterministic (a node's children have higher indices
    // than… — actually the reader assigns ids in construction order), giving `check_unit_defines`'s
    // "earlier declaration"/"first-wins" a stable order. A top-level define is just a node here, scanned
    // once (no double-count). The recognized-top-level-form set (`TOP_LEVEL_FORMS` / `scan_top_level`)
    // still handles a top-level `Unit.define` separately for the "unknown top-level" decline — this scan
    // only feeds the conversion table + the known-unit set + the value-reduction lookup.
    for i in 0..ast.structure.len() {
        let item = StructId(i as u32);
        if let Some(args) = unit_member_call(ast, item, "define")
            && args.len() == 4
            && let Some(name) = ast.as_sym(args[0])
            && let Some(num) = ast.as_int(args[2]).and_then(|v| v.to_i128())
            && let Some(den) = ast.as_int(args[3]).and_then(|v| v.to_i128())
        {
            out.push((name.to_string(), args[1], num, den));
        }
    }
    out
}

/// Scan the program's `(bind Effect "cadenza:pkg/iface")` top-level directives into the effect→peer-
/// contract default map (U2, the effects-unification of cross-component interop). Each binds an escaping
/// effect NAME to a peer interface STRING — the component-scope default route for that effect. A malformed
/// `(bind …)` (wrong arity, non-name effect, non-string interface) is skipped (a diagnostic elsewhere);
/// first-wins on a duplicate effect name. Empty for a program with no `(bind …)`.
fn scan_effect_bindings(ast: &Arenas) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for item in top_items(ast) {
        if let Some(tail) = ast.as_form(item, "bind")
            && tail.len() == 2
            && let Some(effect) = ast.as_name(tail[0])
            && let Some(iface) = ast.as_str(tail[1])
        {
            out.entry(effect.to_string())
                .or_insert_with(|| iface.to_string());
        }
    }
    out
}

/// The argument list of a `((. Unit KEY) arg…)` member-access call at `item`, or `None` if `item` is not
/// that shape. Recognizes the reader's desugaring of `(Unit.KEY arg…)` — the head is a `(. Unit KEY)`
/// list. Used to scan `Unit.define` top-level forms (and to recognize them as modeled forms so they
/// don't decline as "unknown top-level").
fn unit_member_call<'a>(ast: &'a Arenas, item: StructId, key: &str) -> Option<&'a [StructId]> {
    let Struct::List(items) = ast.get(item) else {
        return None;
    };
    let head = *items.first()?;
    // The head must be `(. Unit KEY)`.
    let dot = ast.as_form(head, ".")?;
    if dot.len() == 2 && ast.as_name(dot[0]) == Some("Unit") && ast.as_name(dot[1]) == Some(key) {
        Some(&items[1..])
    } else {
        None
    }
}

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

/// The recognized TOP-LEVEL form heads — the constructs the scan turns into an index entry, PLUS
/// `module-doc` (a benign no-op — see below). A top-level item whose head is NOT one of these is a
/// construct the compiler does not model (e.g. `(pragma …)`), and the whole program DECLINES rather than
/// silently ignoring it and compiling the rest (decline-don't-miscompile — a program with an unmodeled
/// declaration is out of scope, not partially meaningful). Kept in sync with `scan_top_level`'s branches.
///
/// `module-doc` is a file/module-level doc-comment node — a `///` header before a non-documentable form
/// (e.g. an `import`), emitted by the reader so a file header round-trips as `///` rather than a `//`
/// comment. It DECLARES nothing (`scan_top_level` has no branch for it — it is simply skipped), so it
/// produces no index entry; it is listed here ONLY so `unknown_top_forms` recognizes it as legitimate
/// and does not reject it as an unmodeled declaration. It is inert at every later stage (a no-op form).
///
/// `world` is an in-source `(world …)` TARGET-WORLD declaration — the paren-form twin of an external
/// KIND_WIT_WORLD artifact. It follows the SAME recognize-register-nothing pattern as `module-doc`
/// (`scan_top_level` has no branch, so it registers no def/type/effect and never resolves as a value),
/// but it is NOT inert: `compile` reads it via `top_world_form` and codec-encodes its subtree into
/// `db.wit_world` (when no artifact overrides), so it drives emit-to-match without being a runtime decl.
const TOP_LEVEL_FORMS: &[&str] = &[
    "def",
    "export",
    "type",
    "effect",
    "bind",
    "module-doc",
    "world",
];

/// The declaration/directive keywords a top-level `(head …)` form may legitimately lead with — the
/// closed candidate pool for a "did you mean?" when an unknown top-level head is a plausible TYPO of one
/// (`(exprot f)` → `export`, `(deff …)` → `def`). A superset of [`TOP_LEVEL_FORMS`]: it adds `module`
/// (a grammar head, not in the scan set) and `pragma` (a directive validated separately), because a user
/// mistyping any of these writes a top-level form, and pointing at the intended keyword is the fix. Kept
/// in one place so the suggestion pool cannot drift into naming a keyword the grammar would then reject.
pub const TOP_LEVEL_KEYWORDS: &[&str] = &[
    "def", "export", "type", "effect", "bind", "module", "pragma", "world",
];

/// Substitute a generic newtype's TEMPLATE `Ty` at a concrete instantiation: replace each `Ty::Var(i)`
/// (a declaration parameter's positional slot, planted by `infer::decode_payload_template`) with `args[i]`
/// — the type the sum was instantiated at. A `Var(i)` with `i` past `args` (a malformed/under-applied
/// instantiation) is left as-is (a free var the boundary rejects, not a panic). A template with no vars
/// (a monomorphic newtype) is cloned unchanged. Descends every structural `Ty` that carries inner types.
/// Rewrite an embedded `Ty::Sum` (or `Ty::Nominal`) whose `decl` is an erasable NEWTYPE into its
/// `normalize_sum` form, so a stored `newtype_inner` template agrees with what a query-time reader would
/// decode. Used ONCE at load to fix a template that referenced a newtype not-yet-keyed when it was decoded
/// (the cross-module decl-order case — 9939). Recurses into structural children and into a keyed decl's
/// type-ARGUMENTS, but NOT into a produced `Nominal`'s `inner`: `inner` is the derived one-level unfold
/// `normalize_sum` computes from that decl's OWN stored template (rewritten as its own map entry), so
/// descending it would re-expand a RECURSIVE newtype's `Sum` back-edge forever. Stopping at the node
/// yields exactly the finite shape query-time `normalize_sum` produces. Pure over `&Db` (a map lookup +
/// structural rebuild).
fn normalize_embedded_sums(db: &Db, t: &crate::ty::Ty) -> crate::ty::Ty {
    use crate::ty::Ty;
    match t {
        // A keyed newtype reference (as a raw `Sum` baked by the load-order, or an already-erased
        // `Nominal`) → its canonical `normalize_sum` form, with its args normalized first. `normalize_sum`
        // re-derives `inner` from the decl's own (separately-rewritten) stored template; we do not walk it.
        Ty::Sum { decl, args } | Ty::Nominal { decl, args, .. }
            if db.newtype_inner.contains_key(decl) =>
        {
            let new_args: Vec<Ty> = args
                .iter()
                .map(|a| normalize_embedded_sums(db, a))
                .collect();
            db.normalize_sum(*decl, new_args)
        }
        // A non-newtype sum stays boxed; normalize its args (a generic sum's payload template).
        Ty::Sum { decl, args } => Ty::Sum {
            decl: *decl,
            args: args
                .iter()
                .map(|a| normalize_embedded_sums(db, a))
                .collect(),
        },
        Ty::Nominal { decl, args, inner } => Ty::Nominal {
            decl: *decl,
            args: args
                .iter()
                .map(|a| normalize_embedded_sums(db, a))
                .collect(),
            inner: inner.clone(),
        },
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|e| normalize_embedded_sums(db, e))
                .collect(),
        ),
        Ty::List(e) => Ty::List(Box::new(normalize_embedded_sums(db, e))),
        Ty::Map(k, v) => Ty::Map(
            Box::new(normalize_embedded_sums(db, k)),
            Box::new(normalize_embedded_sums(db, v)),
        ),
        Ty::Set(e) => Ty::Set(Box::new(normalize_embedded_sums(db, e))),
        Ty::Fn(p, r) => Ty::Fn(
            Box::new(normalize_embedded_sums(db, p)),
            Box::new(normalize_embedded_sums(db, r)),
        ),
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), normalize_embedded_sums(db, t)))
                .collect(),
        )),
        Ty::Qty { inner, unit } => Ty::Qty {
            inner: Box::new(normalize_embedded_sums(db, inner)),
            unit: unit.clone(),
        },
        other => other.clone(),
    }
}

fn subst_template_vars(template: &crate::ty::Ty, args: &[crate::ty::Ty]) -> crate::ty::Ty {
    #[cfg(test)]
    SUBST_TEMPLATE_VARS_VISITS.with(|c| c.set(c.get() + 1));
    use crate::ty::Ty;
    match template {
        Ty::Var(i) => args.get(*i as usize).cloned().unwrap_or(Ty::Var(*i)),
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| subst_template_vars(t, args)).collect()),
        Ty::List(elem) => Ty::List(Box::new(subst_template_vars(elem, args))),
        Ty::Map(k, v) => Ty::Map(
            Box::new(subst_template_vars(k, args)),
            Box::new(subst_template_vars(v, args)),
        ),
        Ty::Set(e) => Ty::Set(Box::new(subst_template_vars(e, args))),
        Ty::Fn(p, r) => Ty::Fn(
            Box::new(subst_template_vars(p, args)),
            Box::new(subst_template_vars(r, args)),
        ),
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), subst_template_vars(t, args)))
                .collect(),
        )),
        // A scalar / sum / nominal / qty template leaf carries no param slot to fill (a template that
        // reached a `Ty::Sum` was rejected by the sum-free guard, so it never lands here) — clone as-is.
        other => other.clone(),
    }
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
    fn scan_effect_decl_reads_the_at_resource_marker_sibling() {
        // The `@resource` marker lifts (hash-clean, stripped from the op TYPE) to a `(resource <idx>)`
        // decl-level sibling as the op node's 3rd child (v-syntax 2026-08-13). scan_effect_decl reads it
        // into OpDecl.resource. Here: op `send` marks param 0 as the resource → resource=Some(0); op `poll`
        // has no marker → resource=None. (The sexpr form is what rcdzc scans — the ML `@resource` desugars
        // to this `(op <name> <clean-type> (resource N))`.)
        let ast = crate::testkit::parse(
            "(module m \
               (effect E (op send (-> Bytes Bytes Unit) (resource 0)) \
                         (op poll (-> Unit Bytes))) \
               (def (f) 0) (export f))",
        );
        // Find the (effect …) top-level item + scan it.
        let items = ast.as_form(ast.root, "module").expect("module");
        let eff_item = items
            .iter()
            .copied()
            .find(|&it| ast.as_form(it, "effect").is_some())
            .expect("effect item");
        let decl = scan_effect_decl(&ast, eff_item).expect("scans");
        assert_eq!(decl.ops.len(), 2);
        assert_eq!(decl.ops[0].name, "send");
        assert_eq!(
            decl.ops[0].resource,
            Some(0),
            "the (resource 0) sibling → OpDecl.resource = Some(0)"
        );
        assert_eq!(decl.ops[1].name, "poll");
        assert_eq!(
            decl.ops[1].resource, None,
            "an op with no marker → resource = None"
        );
        // The op TYPE is still read (the marker is a SIBLING, not part of the type).
        assert!(
            decl.ops[0].ty.is_some(),
            "the op type occurrence is still captured alongside the resource sibling"
        );
    }

    #[test]
    fn def_by_name_finds_the_definition() {
        let (ast, _) = scalar_program();
        let db = Db::load(ast);
        assert_eq!(db.def_by_name("main"), Some(0));
        assert_eq!(db.def_by_name("nope"), None);
    }

    /// `@tag("…")` — the call-style annotation `(@ (tag "s") (def …))` — records its string against the
    /// annotated def, readable by `tags_of`; a def with no `@tag` has none; stacked `@tag`s accumulate;
    /// and a `@tag` composes with `@test` (both annotations on one def take effect). Pins the
    /// `strip_annotations` app-name recognition + the `Db::tags` map that backs `cdz test --tag`.
    #[test]
    fn tag_annotation_records_string_tags_by_def() {
        let ast = crate::testkit::parse(
            "(do \
               (@ (tag \"slow\") (def (a) 1)) \
               (@ (tag \"slow\") (@ (tag \"net\") (def (b) 2))) \
               (@ test (def (c) 3)) \
               (export a))",
        );
        let db = Db::load(ast);
        let tags = |name: &str| db.tags_of(db.def_by_name(name).expect("def")).to_vec();
        // A single `@tag` records exactly its string.
        assert_eq!(tags("a"), vec!["slow".to_string()]);
        // Stacked `@tag`s accumulate (order: innermost annotation is stripped first, so "net" then "slow"
        // as the loop unwraps — assert as a set-membership, order is an implementation detail).
        let b = tags("b");
        assert!(
            b.contains(&"slow".to_string()) && b.contains(&"net".to_string()) && b.len() == 2,
            "both tags recorded: {b:?}"
        );
        // A def with `@test` but no `@tag` has no tags (a tag is independent metadata).
        assert!(tags("c").is_empty(), "an untagged def has no tags");
    }

    /// `@exhaustive` records its def in the exhaustive set (readable by `Db::is_exhaustive`) AND makes it a
    /// test (so `test_defs` hoists it even without an explicit `@test`). A def not so annotated — a plain
    /// `@test`, or an ordinary def — is NOT exhaustive. Pins the `strip_annotations` recognition + the
    /// `Db::exhaustive` map that backs the `cdz test` exhaustive-enumeration path.
    #[test]
    fn exhaustive_annotation_marks_the_def_and_makes_it_a_test() {
        let ast = crate::testkit::parse(
            "(do \
               (@ exhaustive (def (e (: a Bool)) unit)) \
               (@ test (def (t) 1)) \
               (def (plain) 2) \
               (export plain))",
        );
        let db = Db::load(ast);
        let idx = |name: &str| db.def_by_name(name).expect("def");
        let tests = db.test_defs();
        let is_test = |name: &str| tests.contains(&idx(name));
        // The `@exhaustive` def is exhaustive AND a test.
        assert!(db.is_exhaustive(idx("e")), "@exhaustive def is exhaustive");
        assert!(is_test("e"), "@exhaustive def is also a test");
        // A plain `@test` is a test but NOT exhaustive.
        assert!(is_test("t"), "@test def is a test");
        assert!(
            !db.is_exhaustive(idx("t")),
            "a plain @test is not exhaustive"
        );
        // An unannotated def is neither.
        assert!(
            !db.is_exhaustive(idx("plain")),
            "an ordinary def is not exhaustive"
        );
        assert!(!is_test("plain"), "an ordinary def is not a test");
    }

    /// 9939 (cross-module / decl-order newtype-erasure): a newtype whose payload references ANOTHER
    /// erasable newtype declared LATER must still store an ERASED (`Ty::Nominal`) inner for that field, not
    /// the pre-erasure boxed `Ty::Sum`. `newtype_underlying` decodes each decl's template in `type_decls`
    /// order, so a decl processed BEFORE the newtype it references (`Note` before `Pitch` here, exactly the
    /// order a cross-module link splices an importing file ahead of its import) decoded that field while
    /// the referenced decl was not yet a `newtype_inner` key, baking a raw `Ty::Sum{Pitch}`. Left raw, a
    /// projection of `Note`'s `Pitch` field reads a `Ty::Sum` and the backend treats it as a heap handle
    /// though the value is the erased `Int` — an invalid-wasm miscompile. The post-population re-normalize
    /// pass fixes the stored template so EVERY reader agrees. Pins that `Note`'s inner tuple carries the
    /// `Pitch` field as `Nominal { inner: Int }`, never a bare `Sum`.
    #[test]
    fn a_newtype_referencing_a_later_declared_newtype_stores_an_erased_inner() {
        // `Note` is declared BEFORE `Pitch`, so it is decoded first — the decl order that baked the bug.
        let ast = crate::testkit::parse(
            "(do \
               (type Note (Note Pitch Int64)) \
               (type Pitch (Pitch Int64)) \
               (def (main) 0) (export main))",
        );
        let db = Db::load(ast);
        let occ_of = |name: &str| {
            db.type_decls
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("type {name}"))
                .occ
        };
        // Pitch itself erases to a nominal Int.
        let pitch_inner = db
            .newtype_inner
            .get(&occ_of("Pitch"))
            .expect("Pitch is an erasable newtype");
        assert!(
            matches!(pitch_inner, crate::ty::Ty::Int(_)),
            "Pitch erases to a nominal Int, got {pitch_inner:?}"
        );
        // Note's stored inner is a tuple whose FIRST element is the Pitch field — and it must be the
        // ERASED `Nominal`, not a raw `Sum` (the 9939 miscompile).
        let note_inner = db
            .newtype_inner
            .get(&occ_of("Note"))
            .expect("Note is an erasable newtype");
        let crate::ty::Ty::Tuple(elems) = note_inner else {
            panic!("Note erases to a 2-tuple, got {note_inner:?}");
        };
        match &elems[0] {
            crate::ty::Ty::Nominal { decl, inner, .. } => {
                assert_eq!(*decl, occ_of("Pitch"), "field 0 is the Pitch nominal");
                assert!(
                    matches!(inner.as_ref(), crate::ty::Ty::Int(_)),
                    "Pitch nominal's inner is Int, got {inner:?}"
                );
            }
            other => panic!(
                "Note's Pitch field must be an ERASED Nominal, not a raw Sum (9939), got {other:?}"
            ),
        }
    }

    /// Assert no `Ty::Sum` naming a KEYED newtype survives anywhere in a stored template — the 9939
    /// invariant, generalized: `normalize_embedded_sums` must reach an erasable-newtype reference at ANY
    /// structural depth (a field that is a `List`/`Tuple`/`Map` of the newtype), not just a top-level
    /// field. A residual such `Sum` is the miscompile (a projection would read it as a heap handle). Does
    /// NOT descend a `Nominal.inner` — that hint is derived from the referenced decl's own (separately
    /// asserted) template, and descending it would loop on a recursive newtype, exactly as the pass does
    /// not descend it.
    fn assert_no_keyed_sum(db: &Db, t: &crate::ty::Ty, ctx: &str) {
        assert_no_keyed_sum_guarded(db, t, ctx, &mut std::collections::HashSet::new());
    }

    // Walks `t` asserting NO raw `Ty::Sum` naming a keyed newtype survives — including TRANSITIVELY inside a
    // `Ty::Nominal.inner` (PR#659: the wasm backend recurses into `inner` via `valtype_of`/`is_heap_type`, so
    // a raw keyed `Sum` there is a real miscompile). `visited` holds the `Nominal` decls already descended so
    // a RECURSIVE newtype's `inner` (whose back-edge re-references its own decl) does not loop.
    fn assert_no_keyed_sum_guarded(
        db: &Db,
        t: &crate::ty::Ty,
        ctx: &str,
        visited: &mut std::collections::HashSet<StructId>,
    ) {
        use crate::ty::Ty;
        match t {
            Ty::Sum { decl, args } => {
                let name = db
                    .type_decl_by_occ(*decl)
                    .map(|d| d.name.as_str())
                    .unwrap_or("<sum>");
                assert!(
                    !db.newtype_inner.contains_key(decl),
                    "a raw Ty::Sum naming the KEYED newtype `{name}` survives in {ctx} — 9939 residual"
                );
                for a in args.iter() {
                    assert_no_keyed_sum_guarded(db, a, ctx, visited);
                }
            }
            Ty::Nominal {
                decl, args, inner, ..
            } => {
                for a in args.iter() {
                    assert_no_keyed_sum_guarded(db, a, ctx, visited);
                }
                // Descend `inner` too (a transitive keyed `Sum` here IS a miscompile), guarding against a
                // recursive newtype's self-referential back-edge with the visited-decl set.
                if visited.insert(*decl) {
                    assert_no_keyed_sum_guarded(db, inner, ctx, visited);
                    visited.remove(decl);
                }
            }
            Ty::Tuple(elems) => elems
                .iter()
                .for_each(|e| assert_no_keyed_sum_guarded(db, e, ctx, visited)),
            Ty::List(e) | Ty::Set(e) => assert_no_keyed_sum_guarded(db, e, ctx, visited),
            Ty::Map(k, v) => {
                assert_no_keyed_sum_guarded(db, k, ctx, visited);
                assert_no_keyed_sum_guarded(db, v, ctx, visited);
            }
            Ty::Fn(p, r) => {
                assert_no_keyed_sum_guarded(db, p, ctx, visited);
                assert_no_keyed_sum_guarded(db, r, ctx, visited);
            }
            Ty::Record(fields) => fields
                .values()
                .for_each(|f| assert_no_keyed_sum_guarded(db, f, ctx, visited)),
            Ty::Qty { inner, .. } => assert_no_keyed_sum_guarded(db, inner, ctx, visited),
            _ => {}
        }
    }

    /// 9939 hardening: the re-normalize must reach a newtype reference NESTED inside a structural field
    /// (`(type Chord (Chord (List Pitch) Int64))` where `Pitch` is declared LATER), not only a top-level
    /// field — a `List Pitch` element that stayed a raw `Ty::Sum{Pitch}` would miscompile a projection of
    /// the list's elements exactly as the flat field did. Pins that NO keyed-newtype `Sum` survives at any
    /// depth of `Chord`'s stored template.
    #[test]
    fn a_newtype_field_nesting_a_later_newtype_erases_at_depth() {
        let ast = crate::testkit::parse(
            "(do \
               (type Chord (Chord (List Pitch) Int64)) \
               (type Pitch (Pitch Int64)) \
               (def (main) 0) (export main))",
        );
        let db = Db::load(ast);
        let chord = db
            .type_decls
            .iter()
            .find(|t| t.name == "Chord")
            .expect("Chord");
        let inner = db
            .newtype_inner
            .get(&chord.occ)
            .expect("Chord is an erasable newtype");
        // The whole stored template is free of any raw Sum naming a keyed newtype — including the
        // `List`-element Pitch reference.
        assert_no_keyed_sum(&db, inner, "Chord's stored template");
        // And concretely: element 0 is a `List` whose element is the erased Pitch nominal.
        let crate::ty::Ty::Tuple(elems) = inner else {
            panic!("Chord erases to a 2-tuple, got {inner:?}");
        };
        let crate::ty::Ty::List(elem) = &elems[0] else {
            panic!("Chord field 0 is a List, got {:?}", elems[0]);
        };
        assert!(
            matches!(elem.as_ref(), crate::ty::Ty::Nominal { .. }),
            "the list element is the erased Pitch nominal, got {elem:?}"
        );
    }

    /// 9939 hardening: a SINGLE-VARIANT RECURSIVE newtype that also references a LATER newtype
    /// (`(type Seq (Seq (Option (Tuple Pitch Seq))))` before `Pitch`) must LOAD (the re-normalize
    /// terminates) and store an erased template with no residual keyed-newtype `Sum`. This is the case the
    /// pass's "do not descend a produced `Nominal.inner`" rule protects: descending it would re-expand
    /// `Seq`'s self-referential back-edge forever. `Db::load` returning at all proves termination (a test
    /// that hangs fails by timeout); the scan proves the `Pitch` reference still erased.
    #[test]
    fn a_recursive_single_variant_newtype_referencing_a_later_newtype_loads_and_erases() {
        let ast = crate::testkit::parse(
            "(do \
               (type Seq (Seq (Option (Tuple Pitch Seq)))) \
               (type Pitch (Pitch Int64)) \
               (def (main) 0) (export main))",
        );
        // Termination: this returns rather than diverging (the `Nominal.inner` no-descend guard).
        let db = Db::load(ast);
        let occ_of = |name: &str| {
            db.type_decls
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("type {name}"))
                .occ
        };
        let seq_inner = db
            .newtype_inner
            .get(&occ_of("Seq"))
            .expect("Seq is an erasable newtype");
        // No raw keyed-newtype Sum survives — the `Pitch` reference nested under `Option`/`Tuple` erased,
        // and `Seq`'s own self-reference (a boxed `Sum{Seq}` back-edge — Seq is keyed) is likewise
        // normalized to its `Nominal` at every occurrence in the stored template.
        assert_no_keyed_sum(&db, seq_inner, "Seq's stored template");
    }

    /// PR#659 (Copilot): a TRANSITIVE newtype chain A→B→C declared in REVERSED dependency order (A first)
    /// must have NO raw keyed `Ty::Sum` left inside a `Nominal.inner` at ANY depth of A's stored template.
    /// A single re-normalize pass baked `Nominal{decl=B, inner=<raw Sum{C}>}` into A (B's own template was
    /// still raw when A was rewritten), and the wasm backend recurses into `inner` (`valtype_of`/
    /// `is_heap_type`) → the erased-newtype-read-as-boxed-handle miscompile (invalid wasm) at transitive
    /// depth. The fixpoint iteration drives every `inner` clean regardless of declaration order. Uses the
    /// `inner`-descending `assert_no_keyed_sum` (cycle-guarded) so a residual at depth is actually caught.
    #[test]
    fn a_transitive_newtype_chain_in_reversed_order_erases_at_every_inner_depth() {
        // A wraps B wraps C wraps Int64, declared A-FIRST (reversed dependency order — the failing case).
        let ast = crate::testkit::parse(
            "(do \
               (type A (A B Int64)) \
               (type B (B C Int64)) \
               (type C (C Int64)) \
               (def (main) 0) (export main))",
        );
        let db = Db::load(ast);
        let occ_of = |name: &str| {
            db.type_decls
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("type {name}"))
                .occ
        };
        // A's stored template — walked WITH `Nominal.inner` descent — must carry no raw keyed `Sum` at any
        // transitive depth (the `B`-nominal's inner must not hold a raw `Sum{C}`).
        let a_inner = db
            .newtype_inner
            .get(&occ_of("A"))
            .expect("A is an erasable newtype");
        assert_no_keyed_sum(&db, a_inner, "A's stored template (transitive)");
    }

    #[test]
    // The `typeval_of` READ-THROUGH MEMO (CDZ0999 fix) must never cache a PRE-normalization `Ty::Sum` for a
    // keyed newtype — the exact staleness the load-time evict + `typeval_memo_live` gate exist to prevent.
    // A whole-program reduction of every node fills `db.typeval`; scan the filled column and assert NO entry
    // holds a raw keyed `Sum` (which the backend would read as a heap handle → the 9939 miscompile family
    // that the un-memoized path already dodged via the post-load evict). Also asserts the memo is SOUND: a
    // second read equals the first (the fill is idempotent, exercised by the double `typeval_of`).
    fn the_typeval_memo_never_caches_a_pre_normalization_sum_for_a_keyed_newtype() {
        // A newtype whose field references a LATER-declared newtype — the decl order that baked a raw `Sum`
        // into the pre-normalization reduction. Annotate a value with the full type so its annotation node
        // is reduced by `typeval_of` and memoized.
        let ast = crate::testkit::parse(
            "(do \
               (type Note (Note Pitch Int64)) \
               (type Pitch (Pitch Int64)) \
               (def (mk (: p Pitch)) (Note p 1)) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        assert!(
            db.typeval_memo_live,
            "the memo must be live after load completes"
        );
        // Reduce EVERY user node twice — fills the memo, and the second pass reads it back. Each read must
        // agree with the first (idempotent memo), and no reduction may panic.
        let n = db.user_node_count;
        for i in 0..n {
            let id = StructId(i);
            let first = crate::eval::typeval_of(&mut db, id);
            let second = crate::eval::typeval_of(&mut db, id);
            assert_eq!(first, second, "memoized typeval_of read must be idempotent");
        }
        // Scan the filled memo: NO cached type-value may be a raw `Ty::Sum` naming a keyed newtype (the
        // pre-normalization boxed form). Every erasable-newtype reference must be the `Nominal` form.
        for i in 0..n {
            let id = StructId(i);
            if let crate::arena::Slot::Filled(t) = db.typeval.get(id) {
                assert_no_keyed_sum(&db, t, "a memoized typeval_of result");
            }
        }
    }
}
