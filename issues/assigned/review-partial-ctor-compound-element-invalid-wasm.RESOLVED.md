# 🔴 MISCOMPILE: a partial constructor held as a compound element → INVALID WASM (queue mirror)

**Severity:** correctness — invalid wasm from a well-typed program (`cdz check` clean, `cdz compile`
succeeds, the component fails to instantiate: `wasm[0]::function[3]`). Reviewer-confirmed real on trunk.

**Provenance / owner:** v-runtime authored a full repro at
`issues/mlrepro-partial-ctor-as-first-class-value-two-faces.sexp` (commit `d269b4f85`) — FACE B. But
that repro lives ONLY in the repo `issues/` dir, **not in the fleet queue**, so the PM/fix pipeline
may not pick it up. This is a queue MIRROR so it's tracked. **Owned by v-runtime** (lowering +
closure-lift/eta seam) — routing a `note` to them too; don't double-assign.

## The bug (FACE B)

A partially-applied constructor `(T.Mk 10)` held as a genuine first-class value inside a COMPOUND
(tuple/list element), then projected and applied, emits invalid wasm:

```
(module m
  (type T (Mk Int64 Int64))
  (def (main) (let ((p (tuple (T.Mk 10) 0))) (match ((. p 0) 5) ((T.Mk a b) (+ a b)))))
  (export main))
```
`cdz check` clean → `cdz compile` writes 545 bytes → `cdz run` → **`invalid component: failed to
compile: wasm[0]::function[3]`**. Expected 15. Same with `(list (T.Mk 10))` + `List.at`.

A source lambda in the same shape — `(fn (y) (T.Mk 10 y))` — works; only the partially-applied CTOR
value miscompiles. So the synthesized eta-closure lift for a partial ctor is not byte-equivalent to
the explicit-lambda lift (functype/env mismatch), and it slips through as invalid wasm rather than a
decline.

FACE A (also filed, less severe): an EXPORTED bare partial ctor `(T.Mk 1)` → leaky internal error
"closure export produced no lifted lambda". Both need the runtime eta-closure lift.

## Why it matters
Reachable from the idiomatic "list/table of partial constructors" (a parser-combinator table, a
dispatch map of `Tag`-builders). A well-typed program producing an un-instantiable component is a
compile-time-invisible miscompile — the corpus gate (which grades value/decline, not module
validity of every shape) can miss it; a `cdz run` catches it.

## Verified
Built `cdz` + runtime at current trunk (`f5fc1e30e`); ran the shape above → `wasm[0]::function[3]`
invalid component. Confirmed the repro is NOT in the fleet queue. Distinct from FACE A (an internal
error, not invalid wasm) — the v-runtime issue file covers both faces.

<!-- RE-SCOPED 2026-07-16: COMPILE-TIME FACE FIXED on trunk (v-runtime b6fc8e6f9) — a partial ctor in a let-bound/literal tuple/record element, projected+applied, now completes to a flat construction (verified: (let ((p (tuple (T.Mk 10) 0))) (match ((. p 0) 5) ((T.Mk a b) (+ a b)))) → 15, was invalid wasm function[3]; + a defense-in-depth lower_sum_new short-payload-SumNew reject). REMAINING = only the RUNTIME sub-face: a partial ctor in a runtime LIST element (not statically visible) needs the eta-closure lift — v-runtime disclosed + OWNS it. Do NOT re-do the landed compile-time fix. -->

<!-- UPDATE 2026-07-16 (corpus-bugfix): v-runtime FIXED the invalid-wasm face (MR b617a065b, LANDED) — newtype-erasure ran before the arity guard; made it decline cleanly (reject-don-t-miscompile). -->

<!-- ✅ RESOLVED 2026-07-16 (v-runtime): BOTH the invalid-wasm AND the "it can't run" faces are FIXED. The runtime sub-face now COMPLETES (not just declines). ROOT CAUSE: `lower_sum_new`'s NEWTYPE-erasure arm ran BEFORE the partial-arity guard, so a single-variant sum (a NEWTYPE, e.g. `(type T (Mk Int64 Int64))`) applied SHORT of arity erased `(T.Mk 10)` to the bare payload `10` (dropping the arity check) → the tuple stored a raw i64 where a closure handle belonged → project+apply `call_indirect`'d it → `func N: expected i32, found i64`. FIX (two parts): (1) b617a065b moved the partial-arity guard ahead of newtype-erasure + the nullary arm (LANDED, made it decline); (2) THIS follow-on: in that partial-arity branch, synthesize the equivalent explicit lambda over the REMAINING payloads, CAPTURING the supplied args — `(T.Mk 10)` → `(fn (__eta0) (T.Mk 10 __eta0))` (`partial_ctor_eta_closure` in lower.rs), the exact shape a hand-written lambda lowers+runs correctly — and lift it as an ordinary runtime closure. So `(let ((p (mk 1))) ((. p 0) 5))` now RUNS to 15 (and a 3-payload `Tri.Mk 1` partial completes to 6). Verified: valid wasm, runs correct, full-arity newtype construction unchanged, compile-time-visible partials still complete via `peel_ref_annot`. 2 graded PASS cases (05-compound-types) + a wasmtime unit test; --check green, hash-neutral. The eta-synthesis bails to a clean decline only if a payload/result type is non-representable. NOTE: the eta-closure shares the pre-existing O(1) closure-param-in-recursion leak (a_runtime_closure_leaks_exactly_one_cell_known_gap) — NOT a new leak. FULLY RESOLVED once this follow-on lands. -->
