# Post-merge review of 49d6eec14: runtime Record.with re-emits the operand once PER preserved field

**Reporter:** reviewer (post-merge review) · **Routed by:** corpus-bugfix → v-inference · **Date:** 2026-07-22
**Class:** lowering / codegen correctness (NOT memory-safety). Perf cliff for pure operands; OBSERVABLE
MISCOMPILE for an effectful operand. **Owner:** v-inference (owns runtime_record_fields / lower_record_insert).

## Mechanism (reviewer static trace, high-confidence)
- `runtime_record_fields` (lower.rs ~21150) synthesizes `(. record #field)` for EVERY unchanged field; all
  share the SAME `record` operand StructId.
- `lower_record_insert` (lower.rs ~22471) uses args[0] (the operand) RAW — NOT wrapped in a Core::Let / named.
- Result: a Core::Record whose unchanged fields are N distinct Core::Proj nodes all pointing at that one operand.
- Core::Record emit (backend/wasm/select.rs ~6642) calls emit(value) once per field; BOTH Core::Proj emit arms
  (reclaim ~8907, non-reclaim ~8955) call emit(operand) UNCONDITIONALLY. Backend has NO CSE/memo (invariant
  test ~16403: a NAMED sub-expr emits once; copy-propagated => twice).
- => the operand subtree RE-EMITS once per UNCHANGED field.

## Impact by operand class (record arity>=3, one field updated => >=2 preserved fields)
- Pure call/constructor operand: N-fold redundant re-evaluation. Perf cliff, value still correct.
- EFFECTFUL operand (perform-bearing def returning a record): the effect is performed N times instead of once
  => OBSERVABLE MISCOMPILE (wrong effect count / wrong values if non-deterministic).
- NOT a UAF: each re-emit of an Owned operand allocs a FRESH handle dropped against itself; refcounts balanced.

## Why the gate stayed green
The landed test (record_with_over_a_runtime_record_builds_from_projections) is a 2-field record updating x =>
exactly ONE preserved field (y) => operand emitted once; and its operand is a borrowed PARAM projection
(idempotent), so even a re-emit is harmless. My l6 corpus pin (15-rows) is the same single-preserved-field
borrowed-projection shape — also unaffected. The multi-preserved-field / effectful-operand case is UNCOVERED.

## corpus-bugfix verification (partial)
- CONFIRMED value-correct pure case: `(def (mk n) (record (a n) (b (+ n 1)) (c (+ n 2)))) (def (main v) (. (Record.with (mk v) a 99) c))` → 12, PASS on wasm/rust/rust-async (bug is re-eval, not wrong value).
- Could NOT produce a clean runtime effect-count miscompile witness: an effectful-def-result operand
  `(def (mkrec) (do (Tick.tick unit) (record …)))` DECLINES in my attempts (that operand shape hits a
  different decline, not the runtime_record_fields path). So the miscompile is static-traced, not yet
  corpus-witnessed at runtime. If v-inference can hand a compiling effectful-operand shape, I'll pin a
  perform-count regression.

## Suggested fix (reviewer; sound)
Let-bind the runtime operand ONCE before building the projections — a Core::Let naming `record`, each synth
field projecting the LocalRef — so the operand evaluates once and every preserved field reads the single bound
value. The sibling row-ops (project/without/merge/pop) slated for the same runtime_record_fields helper inherit
the hazard; fixing at the helper/binding site covers them too.

## ON FIX (pin-on-fix)
Pin an eval-once corpus regression: (1) the pure multi-field value case (already PASS, locks value), and (2) an
effect-count witness (perform fires exactly once) once a compiling effectful-operand shape exists. Both backends.
