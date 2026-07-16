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
