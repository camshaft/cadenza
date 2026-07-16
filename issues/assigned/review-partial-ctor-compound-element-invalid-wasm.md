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

---

## ⏳ OWNER UPDATE (v-runtime, 2026-07-16) — COMPOUND-LITERAL face FIXED; only RUNTIME-materialized remains

The COMPOUND-LITERAL sub-face is **FIXED and landed** (trunk `8676d87b9`): a partial ctor in a
compile-time-visible tuple/record literal, projected + applied — `((. (tuple (T.Mk 10) 0) 0) 5)`, the
let-bound tuple, and a record field — now COMPLETES to a flat construction (`peel_ref_annot` follows the
tuple `Proj` / record `Member` into the visible compound to the ctor spine). Verified + pinned by 3
05-compound corpus cases + a store unit test. So the exact repro in this mirror (the `(tuple …)` case)
now compiles + runs to 15.

**STILL OPEN (narrowed):** only a partial ctor stored in a value NOT compile-time-visible — a RUNTIME
list element forced to MATERIALIZE (e.g. threaded through a recursive fn so `List.len` can't fold it
away) → invalid wasm. Root (tick-6 diagnosis): the element `(T.Mk 10)` triggers TWO inconsistent
lowering paths — `lower_sum_new` (an under-arity `Poison` guard, which does NOT surface here) AND
`eta_ctor_closure` (producing a WRONG-ARITY lifted lambda `(env,i64,i64)` instead of the partial
`(env,i64)`) → a malformed `func N`. The real fix reconciles the two paths so a partial ctor lifts at
the CORRECT partial arity; a focused closure-lifting task (not a per-tick fix). ⚠ `cdz compile` does NOT
validate its output — reproduce with the gate / `wasm-tools validate`, not a bare `cdz compile` (which
"succeeds" writing invalid bytes). Full diagnosis in the v-runtime memory sub-index.
