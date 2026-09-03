//! Cost-tiered optimization levels — the `OptLevel` enum and the `PassManager` that gates each pass by
//! its declared tier. This is the BACKEND-INDEPENDENT optimization framework: it runs on the shared Core
//! column ABOVE the backend split, so every backend (rust, wasm, any future one) inherits its passes.
//!
//! The design is `implementation/design/DESIGN-tiered-optimization-levels-rcdzc.md`. The taxonomy is the
//! operator's decision (2026-07-15): Rust-style `O0`/`O1`/`O2`/`O3`, default `O1`.
//!
//! **The one rule (correctness bar).** Every level MUST produce OBSERVABLY-IDENTICAL behavior — only
//! compile time / output speed / size differ, never semantics. A higher level that changes a result is a
//! miscompile (`core-semantics.md` §Observable Behavior Is A Defined Projection Of A Run). This is why a
//! pass declares a MINIMUM level and the manager runs it iff `requested >= min`: raising the level only
//! ever ADDS behavior-preserving transformations.
//!
//! **Why tier-by-construction.** A pass is registered with its `min_level`; the manager runs only the
//! passes whose tier the requested level reaches. Adding a pass to the pipeline forces a tier choice (the
//! registration takes one), so the fast path (`O0`/`O1`) stays fast as passes accumulate — the reason to
//! build this now, before there are many untiered passes to retrofit.
//!
//! This slice lands the LEVEL + MANAGER skeleton (the stable `OptLevel` enum v-cdz-tooling's
//! `--opt-level` flag / manifest profile parses into, and an empty-but-typed pass pipeline). Migrating
//! the existing `lower.rs` folds and lifting the wasm-backend whole-function passes (LICM, global CSE,
//! accumulator introduction) up to Core O2 passes are later slices, each landed with a level-equivalence
//! gate case proving behavior is unchanged at every level on both backends.

use tracing::trace;

// The `OptLevel` enum + its `ALL`/`as_str`/`Display`/`FromStr` impls now live in the shared
// `cadenza-compile-abi` crate (the compile-boundary contract types both `rcdzc` and `cdz` agree on).
// Re-exported here so `crate::opt::OptLevel` and `rcdzc::OptLevel` (lib.rs) stay byte-stable and every
// existing consumer path keeps resolving. The `CorePass`/`PassManager` framework below — which reads
// and refills a live `Db` — is the compiler IMPLEMENTATION that consumes the level, and stays here.
pub use cadenza_compile_abi::OptLevel;

/// One backend-independent optimization pass over the shared Core column. A pass declares the LOWEST
/// [`OptLevel`] at which it runs (`O0` = always-on canonicalization) and transforms the `Db`'s core
/// column in place. Every pass MUST be behavior-preserving at every level (the correctness bar).
///
/// This trait is the seam the migration fills: the existing `lower.rs` folds become `O0`/`O1` passes and
/// the lifted wasm whole-function analyses become `O2` passes, each registered with the manager. It takes
/// `&mut crate::db::Db` so a pass reads/refills the core column through the normal query producers.
pub trait CorePass {
    /// The lowest level at which this pass runs. `requested >= min_level()` gates it in the manager.
    fn min_level(&self) -> OptLevel;
    /// A short stable identifier for tracing / the pass-timing report.
    fn name(&self) -> &'static str;
    /// Transform the core column in place. MUST preserve observable behavior at every level.
    fn run(&self, db: &mut crate::db::Db);
}

/// Sequences the registered [`CorePass`]es and runs those the requested [`OptLevel`] enables. The manager
/// only sequences + tier-gates passes; it does not implement them. Construct with [`PassManager::for_level`]
/// (registers the standard pipeline) and drive with [`PassManager::run`].
pub struct PassManager {
    level: OptLevel,
    passes: Vec<Box<dyn CorePass>>,
}

impl PassManager {
    /// Build the manager for a requested level, registering the standard Core-pass pipeline in canonical
    /// order. Each pass declares its `min_level`; the manager runs only those the requested level reaches,
    /// so a lower level skips the higher-tier passes and stays fast. Every pass is behavior-preserving at
    /// every level (the correctness bar) — the level-equivalence gate (`xtask gate --opt-sweep`) proves it.
    pub fn for_level(level: OptLevel) -> Self {
        let mut pm = PassManager {
            level,
            passes: Vec::new(),
        };
        // O2 — WHOLE-FUNCTION global common-subexpression elimination on the Core IR (backend-independent,
        // so BOTH backends inherit it; the wasm backend's Lir-slot CSE stays as a realization backstop).
        pm.register(Box::new(cse::GlobalCsePass));
        pm
    }

    /// The level this manager runs at.
    pub fn level(&self) -> OptLevel {
        self.level
    }

    /// Register a pass. Kept `pub` so the migration slices can add passes incrementally; the standard
    /// pipeline is assembled in [`PassManager::for_level`].
    pub fn register(&mut self, pass: Box<dyn CorePass>) {
        self.passes.push(pass);
    }

    /// The passes this level ENABLES, in pipeline order — those whose `min_level <= self.level`.
    /// Exposed so a caller / test can see exactly which passes a level selects (and so the pass-timing
    /// report and the level-equivalence gate can enumerate the active set).
    pub fn enabled(&self) -> impl Iterator<Item = &dyn CorePass> {
        self.passes
            .iter()
            .filter(move |p| self.level >= p.min_level())
            .map(|p| p.as_ref())
    }

    /// Run every enabled pass over the core column, in registration order. A pass whose `min_level`
    /// exceeds the requested level is skipped — so `O0` runs only the canonicalizations, `O3` runs the
    /// whole pipeline.
    pub fn run(&self, db: &mut crate::db::Db) {
        for pass in &self.passes {
            if self.level >= pass.min_level() {
                trace!(target: "rcdzc::opt", pass = pass.name(), level = %self.level, "running core pass");
                pass.run(db);
            }
        }
    }
}

/// Global common-subexpression elimination as a Core O2 pass.
///
/// The BACKEND-INDEPENDENT lift of the wasm backend's Lir-slot CSE (`backend/wasm/select.rs`): it names a
/// repeated, trap-free, SCALAR Core subexpression ONCE in a `Core::Let` and points every occurrence at the
/// shared binding, so BOTH backends compute it once instead of re-emitting it. The value-level analysis is
/// shared (`crate::core_analysis`); the wasm backend keeps its Lir-slot realization as a backstop for what
/// this leaves (they no-op on each other's residue). Registered at `O2` (a whole-function analysis).
///
/// **Soundness (the three guards, replicated verbatim from v-wasm-opt's `select.rs` analysis so the two
/// stay identical — a divergence here is a miscompile on BOTH backends):**
/// - (A) SCALAR-ONLY: a candidate's type must be `!is_heap_type` AND `valtype_of(ty).is_some()`. Sharing a
///   HEAP-handle would drift Perceus refcounts — the binding dups once in the prologue while the body may
///   consume the handle per use, so a second use FBIP-mutates a shared handle (the drift `select.rs`'s
///   4696 guard blocks). A non-representable/unit type has no value to thread through a binding.
/// - (B) FRONTIER: a class is shared only if ≥1 member is in the DOMINATING FRONTIER (always evaluated).
///   A trapping subexpression buried in a branch must not be pulled to the top (it would speculate a trap
///   that a run might never reach); a frontier member is evaluated on every path the body is, so binding
///   it once is trap-equivalent.
/// - (C) TRAP-FREE-OR-FRONTIER via the shared analysis + `is_trap_free`: the candidate predicate admits
///   only deterministic, effect-free reads, and the frontier gate handles the trapping-but-always-run case.
///
/// **Post-lowering rewrite (why it is not the during-lowering `kept_bindings` idiom).** This pass runs
/// after the core column is memoized, so a member occurrence is already concrete Core (not a `LocalRef`).
/// `lower::core_of` consults `db.core_override` FIRST for every node and every parent re-reads its children
/// through `core_of`, so an override on a child IS seen by its parents. For a class with representative
/// `rep` (a canonical member) and members `m…`: bind `rep`'s ORIGINAL subexpression once (in a fresh
/// `synth_core` node `rep_init`, a SEPARATE occurrence so overriding `rep` cannot make the initializer
/// self-referential), wrap the body in `Core::Let { (rep, rep_init) … }`, and override every member
/// occurrence — including `rep` itself inside the body — to `Core::LocalRef { binder: rep }`. Classes are
/// bound inner-first (by `subtree_size`) in one sequential `let*` so a nested class's binding precedes an
/// outer class that reads it.
pub(crate) mod cse {
    use super::{CorePass, OptLevel};
    use crate::ast::StructId;
    use crate::core::Core;
    use crate::core_analysis::{
        collect_dominating_frontier, collect_node_refs, core_eq, core_hash_key, is_heap_type,
        licm_children, subtree_size,
    };
    use crate::db::Db;
    use crate::lower::{core_of, synth_core};

    /// The O2 global-CSE pass.
    pub(crate) struct GlobalCsePass;

    impl CorePass for GlobalCsePass {
        fn min_level(&self) -> OptLevel {
            OptLevel::O2
        }
        fn name(&self) -> &'static str {
            "global-cse"
        }
        fn run(&self, db: &mut Db) {
            // Enumerate every emittable body: each top-level / internal def body + each lifted-lambda body.
            // (A malformed def has `body: None` and is skipped.) `db.defs` / `db.lifted` are the same bodies
            // the backends walk, so CSE-ing them here is inherited by every backend.
            let bodies: Vec<StructId> = db
                .defs
                .iter()
                .filter(|d| !d.internal)
                .filter_map(|d| d.body)
                .collect();
            for body in bodies {
                // ELIGIBILITY: only CSE a body built ENTIRELY from CONTEXT-FREE-LOWERING forms (pure scalar
                // + heap CONSTRUCTORS + field READS). The historical pre-lowering MEMO-POISON hazard is gone
                // (the PassManager now runs POST-LAYOUT, so a pass-time `core_of` is a cache hit, not a
                // first-demand). The gate now guards a CSE-CORRECTNESS hazard: the MVP `cse_body` is not
                // scope-aware, so a subtree reading a PATTERN-BINDER (`Match`/`Try`/`SumPayload`) or a
                // CONTINUATION context (`Handle`/`Resume`/`Host`) is not invariant across occurrences —
                // sharing it miscompiles. Heap CONSTRUCTORS (`Tuple`/`List`/`Record`/`Map`/`Bin`) + field
                // READS are admitted: a repeated SCALAR subexpr inside them CSEs soundly (guard (A) keeps the
                // heap handle itself uncse'd; opt-sweep-verified O0..O3-equivalent). Extending to binder-
                // scoped bodies needs scope-aware grouping (a later slice).
                if body_admits_cse(db, body) {
                    cse_body(db, body);
                }
            }
        }
    }

    /// Whether the body's resolved subtree is built ENTIRELY from forms `cse_body` can safely rewrite — the
    /// eligibility gate for this pass.
    ///
    /// HISTORY: originally "pure-scalar only" to dodge a PRE-LOWERING poison (the pass once ran before lazy
    /// lowering, so a pass-time `core_of` first-demanded a context-needing node WITHOUT its lift/handler/
    /// binder context and memo-poisoned `db.core`). That timing hazard is GONE — the PassManager now runs
    /// POST-LAYOUT (compile.rs), where layout has lowered every reachable body top-down, so a pass-time
    /// `core_of` is a cache hit. What remains is a VALUE-CORRECTNESS hazard: the MVP `cse_body` is NOT
    /// scope-aware, so a subtree reading a PATTERN-BINDER (`Match`/`Try`/`SumPayload`) or a CONTINUATION
    /// context (`Handle`/`Resume`/`Host`) is not invariant across occurrences — sharing it miscompiles.
    ///
    /// So the ALLOW-list admits the CONTEXT-FREE-LOWERING forms: pure scalar/boolean/control leaves, heap
    /// CONSTRUCTORS (`Tuple`/`List`/`Record`/`Map`/`Bin` — a repeated SCALAR subexpr inside them CSEs
    /// soundly; guard (A) keeps the heap handle itself uncse'd) and field READS (`MapField`/`RecordField`/
    /// `BinField`, like `Proj`/`Member`). It REJECTS the binder/continuation forms (`Match`/`Try`/
    /// `SumPayload`/`Lambda`/`Handle`/`Resume`/`Host`/arbitrary-`Apply`). Closed ALLOW-list — an unlisted/
    /// future variant rejects (the safe default). Uses `resolved_of`, NOT `core_of`, so the pre-check cannot
    /// poison the memo. A later slice adds SCOPE-AWARE grouping to admit binder-scoped bodies.
    pub(crate) fn body_admits_cse(db: &mut Db, id: StructId) -> bool {
        use crate::resolved::Resolved;
        let ok = match crate::resolve::resolved_of(db, id) {
            // ADMITTED — CONTEXT-FREE-LOWERING forms: pure scalar/boolean/control leaves (`Ref`/`Param`/
            // `Let`/`If`/`And`/`Not`/`Annot`/`Prim` read a param/let-slot or are scalar-structural), field
            // READS (`Proj`/`Member`/`MapField`/`RecordField`/`BinField` — pure reads, no binder context),
            // and heap CONSTRUCTORS (`Tuple`/`List`/`Record`/`Map`/`Bin` — build a fresh heap value; a
            // repeated SCALAR subexpr inside them CSEs soundly, guard (A) keeps the handle uncse'd).
            // REJECTED — the binder/continuation forms whose subtrees are NOT invariant for scope-unaware
            // CSE: `Lambda` (lift), `Handle`/`Host`/`Resume` (effects/continuation), `Try` (fallible-desugar
            // binder), `Match`/`SumPayload` (pattern binders), and arbitrary `Apply` (callee may reach any).
            Resolved::Unit
            | Resolved::Bool(_)
            | Resolved::Int(_)
            | Resolved::Float(_)
            | Resolved::Char(_)
            | Resolved::Str(_)
            | Resolved::SymbolConst(_)
            | Resolved::Prim(_)
            | Resolved::Ref { .. }
            | Resolved::Param { .. }
            | Resolved::And { .. }
            | Resolved::Not { .. }
            | Resolved::If { .. }
            | Resolved::Proj { .. }
            | Resolved::Member { .. }
            | Resolved::MapField { .. }
            | Resolved::RecordField { .. }
            | Resolved::BinField { .. }
            | Resolved::Tuple { .. }
            | Resolved::List { .. }
            | Resolved::Record { .. }
            | Resolved::Map { .. }
            | Resolved::Bin { .. }
            | Resolved::Annot { .. }
            | Resolved::ConstBlock { .. }
            | Resolved::Let { .. } => true,
            // An APPLY is admitted ONLY when it is a PRIM operator application — `(+ a b)`, `(& x 7)`,
            // `(< a b)` etc. `crate::eval::prim_of` follows the head (a bare operator name resolves to a
            // `Ref` into the prelude, not a bare `Prim`) and returns the `Prim` iff this is a built-in
            // operator; a prim-headed apply lowers context-free (no lambda/handler lift). An apply of an
            // ARBITRARY function (`prim_of` is `None`) is REJECTED: the callee's body could reach an
            // effect/lift/contract. (Args are checked by the recursive AST walk below; the scalar-RESULT
            // filter on candidates still decides what actually gets CSE'd.)
            Resolved::Apply { head, .. } => crate::eval::meta_apply_of(db, head)
                .or_else(|| crate::eval::prim_of(db, head))
                .is_some(),
            _ => false,
        };
        if !ok {
            return false;
        }
        match db.ast.get(id).clone() {
            crate::ast::Struct::List(children) => children.iter().all(|&c| body_admits_cse(db, c)),
            crate::ast::Struct::Atom(_) => true,
        }
    }

    /// CSE one function body: find the shareable classes and install the `Core::Let` + `LocalRef` overrides.
    fn cse_body(db: &mut Db, body: StructId) {
        let groups = candidate_groups(db, body);
        if groups.is_empty() {
            return;
        }
        // Bind every class representative in ONE `Core::Let` wrapping the (original) body. Inner-first order
        // (candidate_groups already sorts by subtree_size) makes a nested class's binding precede an outer
        // one that references it — `let*` sequential semantics.
        let body_ty = crate::infer::type_of(db, body);
        let body_core = core_of(db, body);
        let inner = synth_core(db, body_core, body_ty);
        let mut bindings: Vec<(StructId, StructId)> = Vec::with_capacity(groups.len());
        for group in &groups {
            let rep = group[0];
            let rep_ty = crate::infer::type_of(db, rep);
            // A SEPARATE occurrence holding rep's ORIGINAL subexpr — the binding's VALUE, so no member
            // override affects it.
            let rep_core = core_of(db, rep);
            let rep_init = synth_core(db, rep_core, rep_ty.clone());
            // A FRESH binder id for the slot key. It must NOT be any of the member occurrences: the backend
            // keys the `Core::Let` slot by the binder and resolves a `LocalRef { binder }` by that key, so
            // if the binder were a member (which we override to `LocalRef { binder }`) it would resolve to
            // itself — an infinite cycle. A distinct synth node (its core is the value, though only its id
            // is used as the key) keeps the binding value, the slot key, and the reads all distinct.
            let binder = synth_core(db, Core::LocalRef { binder: rep_init }, rep_ty);
            bindings.push((binder, rep_init));
            // Every member occurrence (including `rep`'s own) now reads the shared binding's slot.
            for &member in group {
                db.install_core_override(member, Core::LocalRef { binder });
            }
        }
        db.install_core_override(
            body,
            Core::Let {
                bindings: bindings.into(),
                body: inner,
            },
        );
    }

    /// The shareable CSE classes of `body`, inner-first. A class is a set of `core_eq` occurrences of a
    /// SCALAR, trap-free-shareable subexpression with ≥2 total references and ≥1 dominating-frontier member.
    fn candidate_groups(db: &mut Db, body: StructId) -> Vec<Vec<StructId>> {
        let mut counts: std::collections::HashMap<StructId, u32> = std::collections::HashMap::new();
        let mut order: Vec<StructId> = Vec::new();
        collect_node_refs(db, body, &mut counts, &mut order);
        let mut dominating: std::collections::HashSet<StructId> = std::collections::HashSet::new();
        collect_dominating_frontier(db, body, &mut dominating);

        // Keep the shareable / non-trivial / SCALAR distinct nodes, in first-seen order (determinism).
        let mut cands: Vec<StructId> = Vec::new();
        for id in order {
            if is_trivial(db, id) || !is_cse_candidate(db, id) {
                continue;
            }
            let ty = crate::infer::type_of(db, id);
            // Guard (A): scalar-only — a heap handle drifts rc; a non-representable/unit type has no value.
            if is_heap_type(&ty) || crate::backend::wasm::lir::valtype_of(&ty).is_none() {
                continue;
            }
            // Guard (D) — BODY-ROOT SCOPE. This pass wraps the WHOLE body in ONE `Core::Let` at the body
            // ROOT, so every shared occurrence is EMITTED at the root and reads the root binding. Two things
            // must therefore hold of each candidate OCCURRENCE:
            //  (D1) it must be in the DOMINATING FRONTIER — an always-evaluated, body-root-reachable
            //       position. An occurrence buried inside a `Match` arm / `If` branch / `Let` body is
            //       reached only conditionally AND lives in that inner scope; hoisting it to the root both
            //       speculates it and detaches it from its scope. The frontier is exactly the set of
            //       root-reachable straight-line positions, so requiring frontier membership keeps the
            //       shared occurrence at a position the root binding dominates. (Previously only the CLASS
            //       needed one frontier member — but a NON-frontier member of that class, sitting inside an
            //       arm, was still overridden to read the root binding → CDZ0101 when its arm-scoped
            //       operands weren't live at the root. Gating EACH occurrence closes that.)
            //  (D2) its subtree must read only body-root-live references (PARAMS / consts), never a
            //       `LocalRef` (a source `let`-local / kept binding) or `Captured` (closure env) — a
            //       `Core::Param` inside a match arm can be an ARM binder (not body-root-live), which (D1)
            //       already excludes by position; (D2) additionally rejects an explicit inner-binder read.
            // (A later, more invasive slice could place the sharing `Let` at an enclosing scope to CSE
            // sub-scope-local subexpressions; this MVP shares only body-root-frontier, body-root-closed reads.)
            if !dominating.contains(&id) || !body_root_closed(db, id) {
                continue;
            }
            cands.push(id);
        }

        // Partition into `core_eq` value classes, bucketed by a cheap shallow hash first (so unequal
        // candidates never pairwise-compare — same bound as v-wasm-opt's analysis).
        let mut classes: Vec<Vec<StructId>> = Vec::new();
        let mut by_key: crate::fxhash::FxHashMap<u64, Vec<usize>> =
            crate::fxhash::FxHashMap::default();
        let mut hash_memo: crate::fxhash::FxHashMap<StructId, u64> =
            crate::fxhash::FxHashMap::default();
        for id in cands {
            let key = core_hash_key(db, id, &mut hash_memo);
            let bucket = by_key.entry(key).or_default();
            let mut placed = false;
            for &ci in bucket.iter() {
                if core_eq(db, classes[ci][0], id) {
                    classes[ci].push(id);
                    placed = true;
                    break;
                }
            }
            if !placed {
                bucket.push(classes.len());
                classes.push(vec![id]);
            }
        }

        // Keep a class iff (a) total references ≥2 (a real repeat worth naming) AND (b) guard (B): ≥1 member
        // in the dominating frontier (so binding it to the top is sound on every path).
        let mut groups: Vec<Vec<StructId>> = classes
            .into_iter()
            .filter(|c| {
                c.iter().map(|m| counts[m]).sum::<u32>() >= 2
                    && c.iter().any(|m| dominating.contains(m))
            })
            .collect();
        // Inner-first, so a nested class's `Core::Let` binding precedes an outer class that reads it.
        groups.sort_by_key(|c| subtree_size(db, c[0]));
        groups
    }

    /// Whether every reference in the subtree at `id` is live at the BODY ROOT — i.e. it reads only
    /// `Param`s / constants, never a `LocalRef` (a source `let`-local or kept binding) or a `Captured`
    /// (closure env cell). Such a subtree can be hoisted into the body-root `Core::Let` this pass builds;
    /// a subtree reading an inner binder cannot (its slot is unbound at the root → CDZ0101 / "no local
    /// slot"). Walks the full VALUE-position subtree via `licm_children` (which descends the sub-scope
    /// children too), so it catches an inner-binder read at any depth — e.g. a `Proj`/`SumPayload` over a
    /// match-arm-bound scrutinee, or an operand nested in an `if` branch.
    fn body_root_closed(db: &mut Db, id: StructId) -> bool {
        match core_of(db, id) {
            // The two reference leaves that name an INNER binder — disqualifying.
            Core::LocalRef { .. } | Core::Captured { .. } => false,
            _ => licm_children(db, id)
                .into_iter()
                .all(|c| body_root_closed(db, c)),
        }
    }

    /// A node too trivial to name: a bare parameter or constant is already a single read at each use, so
    /// binding it would only add an indirection. (`LocalRef` is NOT trivial here — it is a shareable leaf.)
    fn is_trivial(db: &mut Db, id: StructId) -> bool {
        matches!(
            core_of(db, id),
            Core::Param { .. } | Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit
        )
    }

    /// Whether `id` is a PURE, DETERMINISTIC scalar computation whose sharing is observably identical to
    /// recomputing it — v-wasm-opt's `is_cse_shareable` (`select.rs`) with the ONE difference that a
    /// `Core::LocalRef` IS shareable here: their pass hoists BEFORE the body (where a let-local's slot is
    /// unbound), but THIS pass introduces the `Core::Let` IN PLACE, so a reference to an enclosing let-local
    /// is live at the binding point. Every other arm is identical (a divergence would let the two CSEs
    /// disagree on what is safe). Determinism/effect-freedom only — the caller applies the scalar filter.
    fn is_cse_candidate(db: &mut Db, id: StructId) -> bool {
        match core_of(db, id) {
            Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit | Core::Param { .. } => true,
            // A `let`-LOCAL reference is NOT shareable: this pass wraps the WHOLE body in one outer
            // `Core::Let`, which sits OUTSIDE any enclosing source `let ((k …))`, so a shared subexpression
            // that reads `k` (a `LocalRef`) would be bound BEFORE `k`'s slot exists → "let-binding
            // reference has no local slot". (My earlier assumption that this pass binds "in place" was
            // wrong — the sharing `Let` is at the body root, not at the inner `let`'s scope.) So a
            // computation OVER a let-local stays put, exactly as v-wasm-opt's hoist-before-body CSE
            // excludes it. Params (slots `0..n`, live for the whole body) remain shareable. Extending CSE
            // to let-local subexpressions needs the sharing `Let` placed at the enclosing let's scope — a
            // later, more invasive slice; this MVP shares only body-root-live (param/const-rooted) reads.
            Core::LocalRef { .. } => false,
            Core::Arith { lhs, rhs, .. }
            | Core::Compare { lhs, rhs, .. }
            | Core::StrCmp { lhs, rhs, .. }
            | Core::FloatCompare { lhs, rhs, .. } => {
                is_cse_candidate(db, lhs) && is_cse_candidate(db, rhs)
            }
            Core::Convert { operand, .. } | Core::Not { operand } | Core::Proj { operand, .. } => {
                is_cse_candidate(db, operand)
            }
            Core::ListLen { operand }
            | Core::BytesLen { operand }
            | Core::StrScalarLen { operand } => is_cse_candidate(db, operand),
            Core::MapSize { map } => is_cse_candidate(db, map),
            Core::SetLen { set } => is_cse_candidate(db, set),
            Core::SumPayload { scrutinee, .. } => is_cse_candidate(db, scrutinee),
            Core::ListAt { list, index, .. } => {
                is_cse_candidate(db, list) && is_cse_candidate(db, index)
            }
            Core::BytesAt { bytes, index, .. } => {
                is_cse_candidate(db, bytes) && is_cse_candidate(db, index)
            }
            Core::MapLookup { map, key, .. } => {
                is_cse_candidate(db, map) && is_cse_candidate(db, key)
            }
            Core::SumExpect { scrutinee, .. } => is_cse_candidate(db, scrutinee),
            Core::If {
                cond, then_, else_, ..
            } => {
                is_cse_candidate(db, cond)
                    && is_cse_candidate(db, then_)
                    && is_cse_candidate(db, else_)
            }
            // NOT a shareable candidate — a NON-LOCAL exit (v-effects `Core::HandleAbort`, CASE 1): its
            // `value` becomes the whole handle's result via a bare wasm `return`. Deduping/naming it (or any
            // node holding it) would move a divergent control-flow point. Explicit reject-set membership (the
            // `_ => false` default already excludes it, but this documents it as a deliberate CSE barrier —
            // paired with the `collect_dominating_frontier` fence that keeps its abort value out of the
            // hoist frontier). Guards the future CASE-2 mid-function conditional abort.
            Core::HandleAbort { .. } => false,
            _ => false,
        }
    }
}

/// The B2 SHARING-AWARE-EMIT pass — the POST-LAYOUT Core-IR seam (greenlit by v-rust-backend as the
/// layout-owner: every `Layout` field is keyed by def-index/module-level with ZERO per-Core-node state, so
/// an intra-body `Core::Let`-bind + `LocalRef`-repoint — no new defs, no signature change — is provably
/// layout-stable; emit computes per-body slots itself so the new node ids need no layout entry). Runs AFTER
/// `layout::compute` (so the Core column is lowered WITH its lift/handler context — the timing the pre-layout
/// `GlobalCsePass` hook cannot give a heap/compound body) and BEFORE `backend::emit`. Binds a shared
/// heap-handle node (produced by `reduce_handle`'s resume-value substitution — the emit-phase re-descent
/// source, cmb1/pom5) ONCE into a `Core::Let` slot with a DISTINCT fresh binder + repoints its K parent
/// edges to `Core::LocalRef`, so the emit-analysis walks see it once (the durable fix for the
/// `mark_binder_dups` O(K^depth) re-descent). See `backend/wasm/DESIGN-sharing-aware-emit-let-slot.md`.
///
/// INSTALL (this slice — v-rb's emit-coupled half of the Option-ii division): consume `b2_bind_plan`
/// (v-core-opt's analysis: detection + heap-share + template-exclusion + bind-safety gates) and, per planned
/// entry, install a `Core::Let` slot at its `scope_node` binding a COPY of the shared node's core to a
/// DISTINCT fresh binder, then repoint the shared id to a `LocalRef` of that binder (the loop below). This is
/// byte-neutral WHERE THE PLAN IS EMPTY and refcount-correct where it fires (the distinct fresh binder makes
/// `collect_row_op_field_dups`'s `bk == bv` guard false → no double-dup). Both-backend full-corpus opt-sweep
/// 0-div O0..O3 (wasm 6179 / rust 6082) — the bind-safety gates (`b2_bind_plan`: fully-solved-type,
/// not-match-scrutinee, value-stability, trap-free OR unconditionally-reached) keep every admitted bind sound.
/// cmb1/pom5/ksc1 are NOT un-hung by this (verified: cmb1 STILL HANGS at O2 on trunk, EXIT=124; NOT a
/// regression — they hung pre-B2). Their re-descent-driving shares are the handler STATE-TUPLE `(c,k,mx)`
/// (`Tuple([Int64;3])`), read K× via `Core::SumPayload` across the arm's branches — the `reduce_handle`
/// continuation destructuring the resume payload — NOT the `(/ …)` divide (which is scalar `Int64`, not a
/// heap candidate). Those tuple reads are genuine SumPayload SCRUTINEES, so P3 (not-a-match-scrutinee)
/// CORRECTLY excludes them: binding the payload-source tuple once behind a `LocalRef` would break the
/// SumPayload extraction (unsound — a Sum-typed-only P3 relax would MISCOMPILE, since the target is
/// Tuple-typed yet a real SumPayload scrutinee). So cmb1's plan is EMPTY → nothing bound → the
/// `mark_binder_dups` re-descent is unmitigated → it hangs. What B2 DID deliver: SOUNDNESS (the rq3/plt2
/// O2/O3 miscompiles + the FIFO/Record.with/Sum-scrutinee install-hazards are gated out) and the
/// effect-state read-once-per-path hang class where shares are admissible. cmb1-FULL (my emit lane,
/// DEFERRED-pending-operator-priority, joint w/ v-core-opt) is NOT a gate refinement: it needs an EMIT-side
/// MEMO on the SumPayload extraction (so the shared tuple's field-reads aren't re-descended per branch) OR
/// per-branch de-sharing — both WITHOUT binding-once (unsound here). Genuinely deeper than the Let-slot
/// mechanism. Non-blocking: cmb1/pom5/ksc1 are breaker PROBES (`.breaker-probes/`, NOT in the gated
/// `spec/semantics` corpus, so they don't red the gate), and the residual failure is a compile HANG at O2,
/// never a wrong answer — no correctness issue, purely an unfinished optimization.
pub(crate) fn run_sharing_aware_emit(
    db: &mut crate::db::Db,
    layout: &crate::layout::Layout,
    opt_level: OptLevel,
    // SCRUTINEE-SHARES-ONLY (v-cadenza-backend co-design, the cadenza-at-O1 install path): when `true`,
    // install ONLY the plan entries whose shared node is a dispatch subject (`b2_bind_plan_scrutinee_only`) —
    // the serialization-forced match-scrutinee subset that is reclaim-safe on an UN-O2-optimized body, EXCLUDING
    // the general shared-heap-node bindings (loop-carried / captured-list) that leak on the cadenza O1 round-trip
    // (v-cadenza census: 06/09/14b regressed 0→N under the full plan at cadenza-O1). The default wasm path passes
    // `false` (full plan) — the O2 body's pass pipeline makes the general bindings reclaim-safe, so those wins stay.
    scrutinee_shares_only: bool,
) {
    // Gated to O2+ (a whole-body sharing analysis), tier-consistent with `GlobalCsePass`.
    if opt_level < OptLevel::O2 {
        return;
    }
    for &def in &layout.order {
        let Some(body) = db.defs[def].body else {
            continue;
        };
        // (A) BARE-ENTRY-PARAM EXPORT GATE (v-core-opt tick bw, blessed by v-rb): SKIP B2 binding for the
        // body of a SINGLE bare export whose entry param is NON-SCALAR (a `List`/`String`/compound). B2's
        // `Core::Let`-wrap of a shared heap node reshapes such a body so `try_bare_entry_param_component`
        // (mod.rs) returns None on the wrapped body (its param-escape/shape analysis expects the RAW body) →
        // main falls through to the "non-scalar entry parameter not emitted on this export path" decline at
        // O2 where O0/O1 (no B2) compile = an O2 level-equivalence VIOLATION (bisected: B2 no-op → 0 divergences,
        // GlobalCsePass exonerated). Excluded here because try_bare_entry_param_component needs the raw body; the
        // lost B2 win on these bodies is a NEGLIGIBLE shared-scalar read (`List.len`/a String/list param read
        // twice), not a costly recompute; the see-through-Let alternative (option B) is deferred as low-ROI
        // (subtle `param_escapes_body` interaction). Every OTHER def's body (helpers, lifted closures) still gets B2.
        if b2_excluded_bare_entry_export(db, layout, def) {
            continue;
        }
        // STEP 3: INSTALL the B2 bind plan (v-rb's emit-coupled half of the Option-ii division). Each entry
        // is a shared heap node the plan ALREADY gated as sound to bind (fully-solved-type + not-a-scrutinee +
        // value-stable PARAM-closed + [trap-free OR its scope_node unconditionally reaches it]; template-shares
        // and conditionally-reached trapping shares excluded — see b2_bind_plan). For each, install a `Core::Let` slot
        // at its `scope_node` (the members' LCA = the nearest enclosing scope, e.g. the arm-body — NOT the
        // fn/handle body, so no handler-emit perturbation) binding a COPY of the shared node's core to a
        // DISTINCT fresh binder, then repoint the shared id to a `LocalRef` of that binder. This collapses the
        // emit-analysis re-descent (`mark_binder_dups` O(K^depth)) — the durable cmb1 fix. Mirrors `cse_body`'s
        // Let-install; sequential per-entry install NESTS correctly when two entries share a `scope_node`
        // (each captures `scope_node`'s CURRENT core as the new Let body → the second wraps the first, let*).
        // See backend/wasm/DESIGN-sharing-aware-emit-let-slot.md.
        let plan = if scrutinee_shares_only {
            crate::core_analysis::b2_bind_plan_scrutinee_only(db, body)
        } else {
            crate::core_analysis::b2_bind_plan(db, body)
        };
        install_b2_bind_entries(db, &plan);
        if !plan.is_empty() {
            trace!(target: "rcdzc::opt", def, entries = plan.len(),
                "sharing-aware-emit: B2 bind plan INSTALLED (shared heap nodes bound into Let slots)");
        }
    }
}

/// Whether `def`'s body is the body of a SINGLE bare export whose entry param is NON-SCALAR — the (A) gate
/// (v-core-opt tick bw, blessed by v-rb): B2's `Core::Let`-wrap breaks `try_bare_entry_param_component`'s
/// raw-body-shape analysis for such an export, declining at O2 what O0/O1 compile (a level-equivalence
/// violation). TRUE iff there is EXACTLY ONE export, it IS this `def`, and at least one of its params has no
/// scalar BOUNDARY valtype (`export_result_valtype` is not `Ok(Some(_))` for a `List`/`String`/closure/
/// compound). Scalar-only-param exports and non-export defs are UNAFFECTED (return false → keep B2).
/// Conservative: over-excluding only forfeits a cheap shared-read opt on the excluded body, never a miscompile.
fn b2_excluded_bare_entry_export(
    db: &mut crate::db::Db,
    layout: &crate::layout::Layout,
    def: usize,
) -> bool {
    if layout.exports.len() != 1 {
        return false;
    }
    let export = &layout.exports[0];
    if export.def != def {
        return false;
    }
    // A param with no scalar BOUNDARY valtype (a heap `List`/`String`/`Bytes`/closure/compound) is the one
    // `try_bare_entry_param_component` lifts specially off the RAW body — the shape B2's Let-wrap breaks. Use
    // the BOUNDARY check (`export_result_valtype`), NOT the core `valtype_of`: a `List`/`String` IS an i32
    // handle in the core (`valtype_of == Some`) but has NO scalar boundary form, so `valtype_of` would miss
    // exactly the List/String entry params that trigger the decline (elc1/icp2/ssc1). A `Unit` param is
    // zero-width/elided and never hits that lift path, so exclude it from triggering the gate (stays tight).
    let ncx = db.name_ctx();
    export.params.iter().any(|(_, ty)| {
        !matches!(ty.strip_nominal(), crate::ty::Ty::Unit)
            && !matches!(
                crate::backend::wasm::serialize::export_result_valtype(ty, &ncx),
                Ok(Some(_))
            )
    })
}

/// Install one B2 bind plan's entries into the core column (the emit-coupled half — v-rb's lane). Split out
/// of `run_sharing_aware_emit` so the install mechanics can be pinned by a white-box unit test on a SYNTHETIC
/// plan (`testkit::parse`/`Db::load` cannot seed a clean heap-share body — see the scalar-repeat test — so the
/// FIRING path is otherwise only reachable through the corpus `--opt-sweep`).
///
/// BIND-ONCE model (what `b2_bind_plan` actually emits today): each entry's `members` is `[shared_id]` — the
/// ONE shared `StructId` itself (see `b2_member_occurrences`, core_analysis.rs:501-506; `b2_bind_plan`:411-413).
/// The loop overrides that single id to `LocalRef{fresh_binder}`, and its K parent EDGES all follow uniformly
/// (they reference `shared_id` by id, so `core_of(shared_id)` now returning `LocalRef` redirects every parent
/// at once). That is why bind-once needs no per-parent work: child-override propagates to all readers.
///
/// NOT a per-branch de-share primitive. A shared node is ONE `StructId` reached by K parent EDGES (NOT K
/// distinct occurrence ids), and `install_core_override(node)` is ALL-OR-NOTHING across those parents. So this
/// loop CANNOT point different parents of the same `shared_id` at different per-branch copies — cmb1-FULL's
/// per-branch de-share needs an EDGE-level rewrite (redirect a specific parent→`shared_id` edge to a per-edge
/// copy), a SEPARATE install (v-rb, in design w/ v-core-opt 2026-08-19). For cmb1's shares specifically — which
/// are `SumPayload` SCRUTINEES (P3) — the de-share copy must be INLINE (a fresh direct value as the parent's
/// operand), NEVER a slotted `LocalRef` (a `LocalRef` `SumPayload` operand re-breaks the rust nested-variant
/// subject resolution P3 protects). So the future per-edge install rewrites `parent`'s child edge to a fresh
/// copy of `shared_id`'s core, with no `Let`/binder — distinct from this bind-once slot install.
pub(crate) fn install_b2_bind_entries(
    db: &mut crate::db::Db,
    plan: &[crate::core_analysis::B2BindPlanEntry],
) {
    for e in plan {
        // rep_init: a fresh node holding the shared node's ORIGINAL core (the binding VALUE), captured
        // BEFORE the member repoint below — it references the shared node's CHILDREN, not the shared id
        // itself, so the id-repoint leaves the original computation intact.
        let shared_core = crate::lower::core_of(db, e.shared_id);
        let rep_init = crate::lower::synth_core(db, shared_core, e.ty.clone());
        // A DISTINCT fresh binder id (the slot key) typed as the shared value: `is_heap_type_for_retain`
        // sees the heap type so `mark_binder_dups` gives the slot the correct dup/drop; distinct from the
        // value so `collect_row_op_field_dups`'s `bk == bv` guard is false (no double-count). Its core is
        // only a key (never dereferenced for a Let binder). A FRESH one PER ENTRY → per-branch slots stay
        // independent (never reuse a binder across entries).
        let fresh_binder = crate::lower::synth_core(
            db,
            crate::core::Core::LocalRef { binder: rep_init },
            e.ty.clone(),
        );
        // The Let BODY: a fresh node holding `scope_node`'s CURRENT core (so a prior same-scope Let is
        // captured as this Let's body → nested let* bindings, never a clobber).
        let scope_ty = crate::infer::type_of(db, e.scope_node);
        let scope_core = crate::lower::core_of(db, e.scope_node);
        let inner = crate::lower::synth_core(db, scope_core, scope_ty);
        db.install_core_override(
            e.scope_node,
            crate::core::Core::Let {
                bindings: vec![(fresh_binder, rep_init)].into(),
                body: inner,
            },
        );
        // Repoint the entry's members to read the slot. Today `members == [shared_id]` (bind-once): overriding
        // that one id redirects all K parent edges uniformly (they reference it by id). The loop is written over
        // a member LIST for generality, but the plan currently supplies exactly the shared id.
        for &m in &e.members {
            db.install_core_override(
                m,
                crate::core::Core::LocalRef {
                    binder: fresh_binder,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The `OptLevel` enum's own unit tests (ordering / default / `FromStr` / `Display` round-trip) live
    // with the type in `cadenza-compile-abi`. The tests below exercise the rcdzc-side `PassManager` /
    // `CorePass` framework that consumes the level.

    // A pass that records whether it ran, so we can assert the manager's tier gating.
    struct MarkerPass {
        min: OptLevel,
        name: &'static str,
    }
    impl CorePass for MarkerPass {
        fn min_level(&self) -> OptLevel {
            self.min
        }
        fn name(&self) -> &'static str {
            self.name
        }
        fn run(&self, _db: &mut crate::db::Db) {}
    }

    #[test]
    fn manager_enables_only_passes_at_or_below_the_level() {
        let mut pm = PassManager::for_level(OptLevel::O1);
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O0,
            name: "canon",
        }));
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O1,
            name: "local-cleanup",
        }));
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O2,
            name: "whole-fn",
        }));
        let enabled: Vec<&str> = pm.enabled().map(|p| p.name()).collect();
        // O1 enables the O0 + O1 passes, not the O2 one.
        assert_eq!(enabled, vec!["canon", "local-cleanup"]);
    }

    #[test]
    fn o0_enables_only_canonicalization() {
        let mut pm = PassManager::for_level(OptLevel::O0);
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O0,
            name: "canon",
        }));
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O1,
            name: "local-cleanup",
        }));
        let enabled: Vec<&str> = pm.enabled().map(|p| p.name()).collect();
        assert_eq!(enabled, vec!["canon"]);
    }

    #[test]
    fn o3_enables_the_whole_pipeline() {
        // Count the STANDARD pipeline `for_level` registers (currently the O2 `global-cse`), then assert
        // that at O3 all four added markers (one per tier) enable ON TOP of it — so the delta is exactly 4,
        // regardless of how many real passes the standard pipeline grows.
        let baseline = PassManager::for_level(OptLevel::O3).enabled().count();
        let mut pm = PassManager::for_level(OptLevel::O3);
        for (min, name) in [
            (OptLevel::O0, "a"),
            (OptLevel::O1, "b"),
            (OptLevel::O2, "c"),
            (OptLevel::O3, "d"),
        ] {
            pm.register(Box::new(MarkerPass { min, name }));
        }
        assert_eq!(pm.enabled().count(), baseline + 4);
    }

    // ── The core-override seam (§9a) — the mechanism a real CorePass uses to rewrite the Core-IR ──
    // These prove: (1) an empty override map leaves `core_of` behavior unchanged (the level-equivalence
    // baseline — the whole corpus gate exercises this path); (2) an installed override WINS over the
    // lowered/memoized form; (3) an IDENTITY override (reinstalling the node's own core) is behavior-
    // preserving. Together they de-risk the seam before any real rewrite pass registers.

    #[test]
    fn empty_override_map_leaves_core_of_unchanged() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        assert!(!db.has_core_overrides(), "fresh Db has no overrides");
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let natural = crate::lower::core_of(&mut db, body);
        // Still no overrides after a normal lowering.
        assert!(!db.has_core_overrides());
        match natural {
            crate::core::Core::ConstInt(ref v) => assert_eq!(v.to_i64(), Some(42)),
            other => panic!("main's body lowers to ConstInt(42), got {other:?}"),
        }
    }

    #[test]
    fn an_installed_override_wins_over_the_lowered_form() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        // Lower it once (fills the column) so we prove the override wins even over a FILLED slot.
        let natural = crate::lower::core_of(&mut db, body);
        assert!(matches!(&natural, crate::core::Core::ConstInt(v) if v.to_i64() == Some(42)));
        // A pass installs a (deliberately distinct) override for this node.
        db.install_core_override(
            body,
            crate::core::Core::ConstInt(crate::ast::IntValue::from_i64(999)),
        );
        assert!(db.has_core_overrides());
        let after = crate::lower::core_of(&mut db, body);
        match after {
            crate::core::Core::ConstInt(ref v) => assert_eq!(
                v.to_i64(),
                Some(999),
                "core_of returns the pass-installed override"
            ),
            other => panic!("expected the override ConstInt(999), got {other:?}"),
        }
    }

    #[test]
    fn an_identity_override_is_behavior_preserving() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let natural = crate::lower::core_of(&mut db, body);
        // An identity pass reinstalls the node's OWN core as its override — the seam is exercised but
        // the result is unchanged (the byte-identical de-risking case the design's slice-1 calls for).
        db.install_core_override(body, natural.clone());
        assert!(db.has_core_overrides());
        let after = crate::lower::core_of(&mut db, body);
        assert_eq!(
            format!("{natural:?}"),
            format!("{after:?}"),
            "an identity override leaves core_of's result unchanged"
        );
    }

    // ── Verification b2: the proof-guided-elision CorePass MECHANISM prototype ──────────────────────
    // The Inc-b opt seam (implementation/design/DESIGN-verification-program-conditions.md §3): a
    // discharged no-overflow `Thm` should let a Core pass drop the overflow guard at the node it
    // licenses. This prototype proves the PASS WIRING end-to-end — iterate a checked-arith node →
    // consult a proof oracle keyed by the node's StructId → install a core override ONLY when the
    // oracle licenses — without yet changing behavior (b2 is corpus-only, behavior-preserving). It
    // installs the node's OWN core (an IDENTITY override) so the query-then-act path runs unchanged.
    //
    // b3 does NOT use an override or a new Core node. Per the settled seam (§3, disjunction), b3 routes
    // the oracle boolean into `lower::provably_no_overflow = arith_provably_in_range(…) OR
    // discharged_no_overflow(db, id)` — v-core-opt's Slice-5 lands that wrapper + a `false` stub, and b3
    // fills the stub body with the compile-time eval. At b3 this identity-override path is DELETED: the
    // existing `arith_provably_in_range` emit sites do the elision once the disjunction is true, on both
    // backends, with no override and no unchecked variant. (The "Slice-2a unchecked node" plan is
    // SUPERSEDED — no node is needed.)
    //
    // The oracle is a boolean stand-in here. In the real pipeline it is a compile-time `eval` of the
    // discharge program (the Cadenza `licenses` predicate pinned in 26-program-conditions.sexp); the
    // Rust side consumes only its boolean, so this prototype's shape (query → Option → install-or-skip)
    // is exactly the production shape with the eval swapped in for the stand-in.

    /// A proof oracle keyed by Core `StructId`: `Some(())` iff a discharged `Thm` licenses eliding the
    /// overflow guard at that node. In production this is a compile-time eval of the discharge program;
    /// the prototype passes an explicit set of licensed node ids.
    struct StubOracle {
        licensed: std::collections::HashSet<crate::ast::StructId>,
    }
    impl StubOracle {
        fn licenses(&self, id: crate::ast::StructId) -> Option<()> {
            if self.licensed.contains(&id) {
                Some(())
            } else {
                None
            }
        }
    }

    /// The proof-guided-elision pass (b2 mechanism prototype). For the target node, if the oracle
    /// licenses it, install an identity override; otherwise leave it. Behavior-preserving in b2. At b3
    /// this pass is REPLACED by the disjunction (the oracle boolean feeds `provably_no_overflow`), so no
    /// override is installed — the existing emit sites elide. See the module comment above.
    struct ProofElisionPass<'a> {
        oracle: &'a StubOracle,
        target: crate::ast::StructId,
    }
    impl<'a> CorePass for ProofElisionPass<'a> {
        fn min_level(&self) -> OptLevel {
            // Proof-guided elision is a higher-tier optimization; it runs from O2 up. (The real
            // registration is a b3 concern; the prototype does not register into the pipeline.)
            OptLevel::O2
        }
        fn name(&self) -> &'static str {
            "proof-guided-elision"
        }
        fn run(&self, db: &mut crate::db::Db) {
            if self.oracle.licenses(self.target).is_some() {
                // b2: identity override (mechanism only; no behavior change). At b3 this whole path is
                // replaced by the disjunction — the oracle boolean feeds `lower::provably_no_overflow`
                // and the existing emit sites elide; no override is installed.
                let natural = crate::lower::core_of(db, self.target);
                db.install_core_override(self.target, natural);
            }
        }
    }

    #[test]
    fn proof_elision_pass_installs_an_override_only_when_the_oracle_licenses() {
        // A LICENSED node: the pass consults the oracle, gets Some, and installs the (identity) override
        // — proving the iterate → query → install wiring, behavior-preserving in b2.
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let natural = crate::lower::core_of(&mut db, body);
        assert!(!db.has_core_overrides(), "no override before the pass runs");

        let mut licensed = std::collections::HashSet::new();
        licensed.insert(body);
        let oracle = StubOracle { licensed };
        ProofElisionPass {
            oracle: &oracle,
            target: body,
        }
        .run(&mut db);

        // The mechanism fired: an override is installed for the licensed node.
        assert!(
            db.has_core_overrides(),
            "a licensed node gets a proof-guided override installed"
        );
        // …and it is behavior-preserving (b2 identity override): core_of is unchanged.
        let after = crate::lower::core_of(&mut db, body);
        assert_eq!(
            format!("{natural:?}"),
            format!("{after:?}"),
            "the b2 proof-elision override is behavior-preserving (identity)"
        );
    }

    #[test]
    fn proof_elision_pass_leaves_an_unlicensed_node_untouched() {
        // An UNLICENSED node (oracle returns None → no discharged Thm): the pass installs NOTHING, so
        // the overflow check STAYS. This is the default-is-always-the-check invariant at the pass level:
        // absence of a proof means no elision, never elision-on-absence-of-disproof.
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let _ = crate::lower::core_of(&mut db, body);

        // Empty oracle: nothing is licensed.
        let oracle = StubOracle {
            licensed: std::collections::HashSet::new(),
        };
        ProofElisionPass {
            oracle: &oracle,
            target: body,
        }
        .run(&mut db);

        assert!(
            !db.has_core_overrides(),
            "an unlicensed node gets NO override — the check stays (default is always the check)"
        );
    }

    #[test]
    fn global_cse_shares_a_repeated_scalar_subexpression_in_a_let() {
        // `(+ (& x 7) (& x 7))` — the scalar `(& x 7)` appears twice, trap-free, dominating-frontier
        // (both operands of `+` are always evaluated). The O2 CSE pass names it once in a `Core::Let` and
        // points both occurrences at the binding, so the body becomes `let t = (& x 7) in (+ t t)`.
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main (: x Int64)) (+ (& x 7) (& x 7))) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let _ = crate::lower::core_of(&mut db, body);
        assert!(!db.has_core_overrides(), "no override before the pass");

        cse::GlobalCsePass.run(&mut db);

        // The body is now a `Core::Let` binding one value, whose body adds two `LocalRef`s of that binding.
        match crate::lower::core_of(&mut db, body) {
            crate::core::Core::Let {
                bindings,
                body: inner,
            } => {
                assert_eq!(
                    bindings.len(),
                    1,
                    "one shared binding for the repeated `(& x 7)`"
                );
                let binder = bindings[0].0;
                match crate::lower::core_of(&mut db, inner) {
                    crate::core::Core::Arith { lhs, rhs, .. } => {
                        for (label, side) in [("lhs", lhs), ("rhs", rhs)] {
                            assert!(
                                matches!(
                                    crate::lower::core_of(&mut db, side),
                                    crate::core::Core::LocalRef { binder: b } if b == binder
                                ),
                                "the {label} operand reads the shared binding, got {:?}",
                                crate::lower::core_of(&mut db, side)
                            );
                        }
                    }
                    other => panic!("expected the Let body to be the `+`, got {other:?}"),
                }
            }
            other => panic!("expected a `Core::Let` sharing the repeat, got {other:?}"),
        }
    }

    #[test]
    fn global_cse_leaves_a_single_use_body_untouched() {
        // No repeat → nothing to share → no override (the pass must not manufacture a binding).
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main (: x Int64)) (+ (& x 7) (& x 3))) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let _ = crate::lower::core_of(&mut db, body);
        cse::GlobalCsePass.run(&mut db);
        assert!(
            !db.has_core_overrides(),
            "distinct subexpressions are not shared — no override installed"
        );
    }

    #[test]
    fn global_cse_does_not_hoist_a_repeat_whose_other_occurrence_is_branch_local() {
        // GUARD D1 WITNESS (the per-OCCURRENCE dominating-frontier gate — `candidate_groups`:
        // `if !dominating.contains(&id) { continue }`). `(+ (& x 7) (if (< x 0) (& x 7) 0))`: the FIRST
        // `(& x 7)` (lhs of `+`) is in the dominating frontier (always evaluated), but the SECOND lives
        // inside the `if`'s then-branch — a CONDITIONAL, non-frontier position. The scope-unaware MVP wraps
        // the WHOLE body in one body-ROOT `Core::Let`, so it may only share occurrences the root binding
        // dominates. The frontier occurrence alone is a class of ONE (the branch-local occurrence is a
        // distinct StructId excluded from the candidate set by D1), so the `sum(counts) >= 2` keep-filter
        // drops it → NO override. This is the exact regression the per-occurrence gate closed: before it,
        // only the CLASS needed one frontier member, and a non-frontier member sitting inside the arm was
        // still overridden to read the root binding → CDZ0101 when the arm-scoped position wasn't live at
        // the root. Contrast `global_cse_shares_a_repeated_scalar_subexpression_in_a_let`, where BOTH
        // occurrences are frontier operands of `+` and the repeat DOES share.
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main (: x Int64)) (+ (& x 7) (if (< x 0) (& x 7) 0))) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let _ = crate::lower::core_of(&mut db, body);
        assert!(!db.has_core_overrides(), "no override before the pass");
        cse::GlobalCsePass.run(&mut db);
        assert!(
            !db.has_core_overrides(),
            "a repeat with only ONE frontier occurrence (the other buried in a branch) is not hoisted to \
             the body root — guard D1 keeps the scope-unaware CSE from speculating/detaching the arm-local \
             occurrence (the CDZ0101 fence)"
        );
    }

    #[test]
    fn body_admits_cse_gates_on_the_context_free_lowering_allow_list() {
        // The eligibility gate (`body_admits_cse`) is the pass's correctness FENCE: it admits ONLY
        // context-free-lowering forms (pure scalar/control leaves, field reads, heap constructors) and
        // REJECTS the binder/continuation forms whose subtrees are not invariant for the scope-UNAWARE
        // MVP `cse_body` (`Match`/`Try`/`SumPayload`/`Lambda`/`Handle`/`Resume`/`Host`/arbitrary-`Apply`).
        // This pins that contract DIRECTLY (independent of the downstream candidate/dominating-frontier
        // filters), so the scope-aware slice that will relax the fence — and #6902's heap-constructor
        // relaxation that widened it — cannot silently regress the admit/reject sets. Two directions on
        // prelude-free bodies (the harness cannot seed module ops; see the guard-A NOTE below):
        // (1) ADMIT — a pure prim-scalar body `(+ (& x 7) (& x 7))` lowers context-free → eligible.
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main (: x Int64)) (+ (& x 7) (& x 7))) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        assert!(
            cse::body_admits_cse(&mut db, body),
            "a pure prim-scalar body is context-free-lowering → admitted"
        );
        // (2) REJECT — a body with an arbitrary (non-prim) function application `(g 5)`: the callee `g`
        //     is a value param, so `prim_of`/`meta_apply_of` are both `None` → the `Apply` arm rejects,
        //     and the whole body is ineligible. The callee's body could reach an effect/lift/continuation
        //     context the scope-unaware `cse_body` cannot prove invariant across occurrences — this is the
        //     tick-cg handler-threaded-miscompile fence, expressed at the body-eligibility level.
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (ap (: g (-> Int64 Int64))) (+ (g 5) (g 5))) (def (main) 0) (export main))",
        ));
        let d = db.def_by_name("ap").expect("def ap");
        let body = db.defs[d].body.expect("ap body");
        assert!(
            !cse::body_admits_cse(&mut db, body),
            "a body with an arbitrary function application is NOT context-free-lowering → rejected"
        );
    }

    // SOUNDNESS-GUARD WITNESS: the `body_admits_cse` eligibility gate must SKIP any body that
    // contains a nested def / lambda / capture. A pre-emit Core pass that `core_of`s such a body poisons
    // `db.core` (the captured-ref fast path reads `db.captured_ref`, populated only at lambda-LIFT time —
    // walking a capturing body pre-lift follows a captured ref to an out-of-scope binding and memoizes a
    // wrong resolution → "parameter reference has no local slot" at emit). So the pass must install NO
    // override for a body with a nested do-local fn, EVEN THOUGH it holds a repeated scalar `(& x 7)` that
    // WOULD be shared in a capture-free body. This pins the gate that closed the 94→34→0 opt-sweep-div
    // debug arc; a regression that dropped the gate would re-poison and decline this program at O2.
    #[test]
    fn global_cse_skips_a_body_that_contains_a_nested_capturing_def() {
        // `inner` CAPTURES the outer sibling local `k` (it reads `k` in `(+ x k)`), so lowering `outer`'s
        // body lifts `inner` and populates `db.captured_ref` only at lift time. The repeated scalar
        // `(& (+ x k) 7)` lives INSIDE that capturing nested fn. The eligibility gate must skip `outer`'s
        // body (it is not pure-scalar — it holds a nested def/capture), else a pre-emit `core_of` over the
        // capturing body follows the captured `k` ref pre-lift and memo-poisons `db.core` → "parameter
        // reference has no local slot" at emit. (An earlier version had `inner` read only its own param,
        // so it wasn't actually capturing — this one genuinely exercises the lambda-lift hazard.)
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (outer (: n Int64)) \
               (do (def k (* n 3)) \
                   (def (inner (: x Int64)) (+ (& (+ x k) 7) (& (+ x k) 7))) \
                   (inner (+ n 1)))) \
             (export outer))",
        ));
        // Run the pass on a FRESH db (no prior `core_of` of `outer`'s body) — this is the faithful order
        // (Copilot #1662): the hazard is that the pass's OWN pre-emit `core_of` over a capturing body is
        // the FIRST demand and poisons `db.core`; pre-lowering the body here would pre-seed the very
        // context the hazard says shouldn't exist yet, weakening the witness. The eligibility gate must
        // reject `outer`'s body BEFORE the pass `core_of`s it, so no override installs and no poison. The
        // pass enumerates the db's own def bodies, so it reaches `outer` without a handle threaded here.
        cse::GlobalCsePass.run(&mut db);
        assert!(
            !db.has_core_overrides(),
            "a body containing a nested capturing def is NOT pure-scalar → the pass must skip it (no \
             override), else core_of over the capturing body poisons db.core (the memo-poison class)"
        );
    }

    // NOTE on the heap-type exclusion (guard A, `candidate_groups`: `is_heap_type(ty) ||
    // valtype_of(ty).is_none()` → skip): NOT unit-tested here, for a HARNESS reason established empirically
    // (Copilot #1662 review + a debug trace). To isolate guard A a witness needs an ELIGIBILITY-clean body
    // whose SOLE `core_eq` repeat is a HEAP-typed candidate. The only such shapes route a heap value
    // through a collection OPERATION (`Bytes.concat`/`Bytes.len`/`List.len`, so the scalar result doesn't
    // become the maximal shared class) — but `crate::testkit::parse` + `Db::load` (the unit harness) does
    // NOT seed the prelude MODULE operations, so `Bytes.concat`/`Bytes.len`/`List.len` resolve as
    // `Poison(Unbound "concat"/"len")` → the body is a Poison, ineligible for a reason UNRELATED to guard
    // A. (Verified: `(Bytes.concat (. r b) (. r b))` and `(Bytes.len (Bytes.concat …))` both trace an
    // "unbound name" Poison at the module-op node in the unit harness, though both compile fine end-to-end.
    // Copilot's Bytes.concat construction is sound in PRINCIPLE — Member/Proj ARE eligible — but the module
    // op it depends on is unresolvable at the unit level.) An earlier `(list x x)` witness "passed" but via
    // the ELIGIBILITY gate (`Resolved::List` rejected), NOT guard A — a wrong-reason test, removed. Rather
    // than seed the full module prelude into a unit fixture (heavy, brittle), guard A is left to its
    // authoritative END-TO-END coverage where module ops resolve: the `--opt-sweep` level-equivalence gate
    // over the whole corpus + v-wasm-opt's `tests::recursion` battery (heap-carrying loops where a
    // wrongly-shared handle drifts Perceus rc into an observable miscompile). The capturing-body gate above
    // IS unit-tested — it needs no module op, so it is cleanly reachable in the harness.

    #[test]
    fn sharing_aware_emit_is_a_noop_at_o1() {
        // The B2 sharing-aware-emit INSTALL is gated to O2+ (a whole-body sharing analysis, tier-consistent
        // with `GlobalCsePass`). At O1 it MUST early-return without touching the core column — else the
        // default-level (`O1`) gate would see a perturbed emit. Pins the O2-gate on `run_sharing_aware_emit`.
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main (: x Int64)) (+ x 1)) (export main))",
        ));
        let layout = crate::layout::compute(&mut db).expect("layout");
        assert!(!db.has_core_overrides(), "no override before the pass");
        run_sharing_aware_emit(&mut db, &layout, OptLevel::O1, false);
        assert!(
            !db.has_core_overrides(),
            "O1 leaves the core column untouched — the B2 install is gated O2+"
        );
    }

    #[test]
    fn sharing_aware_emit_leaves_a_repeated_scalar_untouched() {
        // B2 binds only shared HEAP-handle nodes (the `reduce_handle` resume-value shares); a repeated
        // SCALAR subexpression is `GlobalCsePass`'s domain, NOT B2's. `(+ (& x 7) (& x 7))` has a repeated
        // scalar `(& x 7)` (count≥2) but `is_heap_type(Int64)` is false, so `b2_bind_plan` excludes it →
        // EMPTY plan → the install is a byte-neutral no-op even at O2. Pins the heap-only scope of B2 (a
        // future change that made B2 grab scalar repeats would double-share with GlobalCse — this catches
        // it). The heap-share FIRING path is authoritatively covered by the corpus `--opt-sweep` (see the
        // harness-limitation note above: `testkit::parse`/`Db::load` cannot seed a clean heap-share body).
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main (: x Int64)) (+ (& x 7) (& x 7))) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let layout = crate::layout::compute(&mut db).expect("layout");
        let _ = crate::lower::core_of(&mut db, body);
        run_sharing_aware_emit(&mut db, &layout, OptLevel::O2, false);
        assert!(
            !db.has_core_overrides(),
            "a repeated SCALAR is not a heap share → b2_bind_plan is empty → B2 installs nothing (that \
             repeat is GlobalCse's job, not B2's)"
        );
    }

    #[test]
    fn install_bind_once_repoints_the_shared_id_to_a_fresh_let_slot() {
        // Pins the BIND-ONCE install contract `b2_bind_plan` actually emits: `members == [shared_id]` (the ONE
        // shared `StructId`; see `b2_member_occurrences`), so the install binds a COPY of `shared_id`'s core into
        // a fresh `Let` slot at `scope_node` and overrides `shared_id` itself to `LocalRef{fresh_binder}` — its K
        // parent edges then all read the slot uniformly (they reference it by id). `testkit::parse`/`Db::load`
        // cannot seed a clean heap-share body (so detection can't be unit-tested — the firing path is otherwise
        // only reached via the corpus `--opt-sweep`), but the standalone `install_b2_bind_entries` consumes a
        // plan slice directly, so this feeds a faithful one-entry `members == [shared_id]` plan and asserts:
        // (1) `scope_node` → `Let` with a DISTINCT fresh binder, (2) `shared_id` → `LocalRef` of that binder,
        // (3) the slot's initializer holds `shared_id`'s ORIGINAL core (the copy, not the LocalRef). NOTE this
        // is bind-once only; per-branch de-share (cmb1-FULL) is a SEPARATE edge-rewrite install (node-override
        // is all-or-nothing across a shared node's K parent edges — see the fn doc).
        use crate::core::Core;
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main (: x Int64)) (+ (+ x 1) (+ x 2))) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        // Pick a real child node as the shared id; `scope_node` = body (an ancestor). `members == [shared_id]`.
        let child = *crate::backend::wasm::select::core_child_ids(&mut db, body)
            .first()
            .expect("body has a child node");
        let ty = crate::infer::type_of(&mut db, child);
        let child_core_before = crate::lower::core_of(&mut db, child);
        let plan = vec![crate::core_analysis::B2BindPlanEntry {
            scope_node: body,
            shared_id: child,
            ty,
            members: vec![child], // the real bind-once shape: the shared id IS the (single) member
        }];
        install_b2_bind_entries(&mut db, &plan);

        // (1) scope_node became a `Let` with a fresh binder + initializer, both DISTINCT from the shared id.
        let (binder, rep_init) = match crate::lower::core_of(&mut db, body) {
            Core::Let { bindings, .. } => (bindings[0].0, bindings[0].1),
            other => panic!("scope_node not a Let: {other:?}"),
        };
        assert_ne!(
            binder, child,
            "the slot binder must be a DISTINCT fresh id (not the shared id)"
        );
        assert_ne!(
            rep_init, child,
            "the slot initializer must be a fresh COPY (not the shared id itself)"
        );
        // (2) the shared id now reads the slot — its K parents follow via this single child-override.
        assert!(
            matches!(crate::lower::core_of(&mut db, child), Core::LocalRef { binder: b } if b == binder),
            "shared_id must be repointed to LocalRef of the slot binder"
        );
        // (3) the slot's initializer preserves the ORIGINAL computation (copied before the override), so the
        // value bound is the shared node's own core — not the LocalRef the shared id was just overridden to.
        assert_eq!(
            crate::lower::core_of(&mut db, rep_init),
            child_core_before,
            "the slot initializer holds a copy of shared_id's ORIGINAL core"
        );
    }
}
