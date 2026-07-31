# DESIGN: emit-once / shared-fn as a general DB COLUMN (operator direction, 2026-07-31)

Owner: v-compiler-ml (query-DB + emit change) · v-compiler-perf (profiling/measure) · v-wasm-opt (emit-quality co-measure).
Operator direction (via concierge assign 18952): (1) PRIORITIZE eliminating EMITTED FUNCTIONS (emit-size is the bigger
concern, not just compile-time); (2) do the memoization as a GENERAL DB COLUMN keyed by def-identity + mono-key —
aligned with the query-DB model (item-4) — NOT one-off local caches.

## Problem
rcdzc lower.rs DEFAULT-inlines non-recursive defs (β-reduce). A high-fan-out helper (add-node: 227 call-sites/5L,
node-at: 92, + the parse-db arena cluster empty-tree/def-body-of/param-of, ALL monomorphic) is inlined N× → (a)
node-count N balloons transitively (the emit cliff: collect_dup_sites was O(B×N), now O(N) after 19fba99b2 but N is
still huge), and (b) emitted-fn/binary BLOAT (the operator's bigger concern) — the same body's lowered instrs are
re-emitted at every inline site instead of once.

## Existing mechanism (reuse, don't reinvent)
- `db.inline_never: FxHashSet<StructId>` (db.rs:1863) — a def in this set emits ONCE as a real fn + CCall, instead
  of inlining. TODAY populated statically from the `@inline-never` source annotation (strip_annotations).
- `db.collect_cache: FxHashMap<StructId, …>` (db.rs:1401) — PRECEDENT: a keyed demand-cache column on StructId.
- `type_specializations` / `effect_specializations` already key by `(orig-body, instantiation-key)` (db.rs:234,1727)
  = the exact (def-identity, mono-key) shape the operator wants; the const-param fingerprint (db.rs:1767) is the
  mono-key precedent.

## Proposed design: an `emit_shared` DB column (demand-computed, keyed)
A new column generalizing `inline_never` from a static annotation set to a demand-computed policy decision:

  emit_shared: FxHashMap<(StructId, MonoKey), EmitPlan>
    where MonoKey = the existing specialization/const-param fingerprint (unit for a plain monomorphic def),
          EmitPlan = EmitOnce{funcidx-slot} | Inline   // the emit-once vs inline decision for THIS (def, mono)

- KEY = (def-identity StructId, mono-key). A monomorphic def → one entry (MonoKey=unit); a generic/const-param def
  → one entry PER instantiation (so specialization is preserved: each mono-instance decides independently, and a
  generic never gets blanket-CCall'd — resolves v-compiler-perf's subtlety (b)).
- DEMAND-COMPUTED (item-4 model): emit-plan-of(db, def, mono) checks the column; on MISS computes the plan from a
  POLICY predicate + fills. Policy for EmitOnce: non-recursive AND fan-out ≥ THRESH AND small-body AND
  monomorphic-at-this-key (a generic body only shares within its mono-instance). Else Inline (preserve folding).
- DRIVES EMIT-ONCE: lower/emit consults emit_shared instead of the static inline_never set — an EmitOnce (def,mono)
  emits its body as ONE wasm function (funcidx) + every call-site becomes a Core::Call to it; an Inline stays β-reduced.
  This SUBSUMES the `@inline-never` annotation (annotation → force an EmitOnce entry; the auto-policy fills the rest).

## Impact (to co-measure — the design's success criteria)
- NODE-COLLAPSE (compile-N): add-node's 227 inlined body-copies → 1 emitted fn + 227 calls. Transitive multiply
  removed for the whole parse-db arena cluster = the dominant N-contributors. v-compiler-perf profiles the compile-N drop.
- EMITTED-FN / BINARY SIZE (operator's PRIMARY concern): today N inline copies of add-node's instrs are emitted; with
  EmitOnce, ONE function body. Net emitted-instr reduction ≈ (N-1)×body-size per shared def, MINUS N call-instr
  overhead. v-wasm-opt co-measures the wasm-size + quality delta (a call can lose downstream fold/specialization —
  but the parse-db arena helpers are pure Map ops over the Tree arena, unlikely to lose meaningful folding; MEASURE).
- SPECIALIZATION LOSS: NONE for monomorphic defs (the whole top cluster). A generic def keyed per-mono-instance keeps
  each instantiation's specialization; only same-mono-instance copies share. So no const-dict-erasure regression.

## Rollout (gated slices)
1. Add the `emit_shared` column + demand producer `emit-plan-of` (policy = the monomorphic-safe top cluster first,
   THRESH tuned to add-node/node-at range). Behind measurement (compile-N + emitted-fn count, no behavior change yet).
2. Wire lower/emit to consult emit_shared (EmitOnce → shared fn + Call). Site-set/behavior-preservation verified via
   the db-demand differential pattern (a program's run value + the emitted module's observable behavior UNCHANGED —
   the typed-exact-eq / differential-oracle approach from item-4).
3. v-wasm-opt co-measure the emit-size/quality delta; tune THRESH by fan-out×body-size to maximize emitted-fn
   elimination without folding regression.

## Open questions for the operator/ruling
- THRESH: fan-out cutoff for auto-EmitOnce (add-node=227 obvious; where's the floor? measure the knee).
- Does EmitOnce apply in the PROVIDER-closure emit specifically (the run-src cliff) or globally? (Global helps binary
  size everywhere; provider-only is the narrower cliff fix.) — operator's call on scope.

## SLICE-1 IMPLEMENTATION MAP (operator GREENLIT 2026-07-31, data-driven, provider-closure-only start) — all hooks located in rcdzc:
- COLUMN: add `emit_shared: FxHashMap<(usize /*callee def idx*/, MonoKey), EmitPlan>` to `struct Db` (db.rs, beside `inline_never` ~1863). Slice-1 MonoKey = unit (monomorphic cluster only); EmitPlan = EmitOnce | Inline. Init default in the 2 Db ctors (db.rs ~2637 beside call_sites_by_callee: None, and ~2613).
- PRODUCER: `pub(crate) fn emit_plan_of(&mut self, callee_idx: usize) -> EmitPlan` in `impl Db` (db.rs:1945). Check column; on MISS compute + fill. POLICY (EmitOnce iff ALL): (a) fan-out ≥ THRESH — `self.call_sites_by_callee`[callee_idx].len() (built by infer::build_call_site_index; ensure built/demand it); (b) NON-recursive — `eval::is_recursive(db, body)` == false (body = the def's body occ); (c) small-body — body node-count ≤ SMALL (reuse a node-count walk); (d) monomorphic — body ∉ generic AND ∉ `db.const_params`. Else Inline.
- THRESH start: conservative — fan-out ≥ ~20 captures add-node(227)/node-at(92)/the arena cluster, skips low-fan-out. Tune in slice-3 co-measure.
- SLICE-1 = COMPUTE ONLY: NO emit change. Verify emit_plan_of returns EmitOnce for the arena cluster (add-node/node-at/empty-tree/def-body-of/param-of) + Inline for low-fan-out/recursive/generic defs. Unit-test in rcdzc tests.rs (rust-side, fast — NOT the cml cliff).
- SLICE-2: wire emit to consult emit_plan_of == EmitOnce → route through the EXISTING emit-once path (emit_call_or_specialize, the same inline_never uses) instead of β-inline. Reuse inline_never's machinery.
- SLICE-3: co-measure — ping v-compiler-perf (node-collapse/compile-N, baseline 194K collect-nodes/17-@test) + v-wasm-opt (emit-size/quality). Tune THRESH. item-4 differential (run-value + emitted-module behavior byte-identical) each slice.
- ⚠ this is rcdzc RUST (db.rs/eval.rs/backend), NOT cml lower-db.cdz. Gates via rcdzc's cargo test + the corpus (fast rust-side unit test for slice-1, not the cml emit cliff).
