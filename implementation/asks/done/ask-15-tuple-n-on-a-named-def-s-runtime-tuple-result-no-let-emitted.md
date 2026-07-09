## 15. 🟢 `tuple.N` on a named-def's runtime-tuple result (no `let`) emitted an INVALID component — FIXED 2026-07-07

**Finding.** `tuple.N` applied directly to a **named-def** function's runtime-tuple result, with no
intervening `let`, emits an **invalid component** (`component failed validation`), not a clean decline
or a defined trap. Sharp boundary:
- `(let ((r (dec 4))) (tuple.0 r))` → ✅ works (the Tier-2e fix: the `let`-bound `Local` carries the
  tuple's `Shape`).
- `(tuple.0 ((fn (x) (tuple x 9)) 7))` → ✅ works (a **lambda** result is compile-time-reduced, shape
  statically resolvable; a corpus case already pins this).
- `(tuple.0 (dec 4))` where `dec` is a **named def** → ❌ **INVALID component**.

**Why it matters.** An invalid component is the category the whole two-compilers gate exists to forbid
— strictly worse than a clean decline or a defined trap, and the gate scores it as a FAIL
(disagreement), not a todo. Note the spike's SEED-GAPS Tier 2e records this as "a VALID component that
TRAPS at the renderer" — direct measurement shows **INVALID** (fails wasm validation), so the handoff
under-states the severity; a decline-don't-miscompile violation was recorded as the milder valid-but-traps
state. It is the same lambda-vs-named-def asymmetry that governs HOF inlining (a lambda inlines, a
named-def HOF declines): where compile-time reduction does not reach, the emitter must **decline**, not
emit invalid code.

**Status.** 🟢 **DONE (2026-07-07, seed side) — and it COMPILES, not just declines.**
`(def (main) (tuple.0 (dec 4)))` now runs → 40 (corpus case *"a scalar element is projected DIRECTLY
from a named function's runtime tuple result"* in `05-compound-types.sexp`; gate 514/0, component-check
521/0, ignition byte-identical). Root cause was NOT in `tuple.N` — it was `gen_runtime_ctor`: its
scalar-path decline (`call_base == 0`) was gated on **all elements being const**, so a tuple with a
runtime element (`(tuple (* n 10) 9)`, `n` a param) emitted `arr-alloc`/`box-int` into an import-free
scalar module → INVALID. Fix: `gen_runtime_ctor` now declines UNCONDITIONALLY on the scalar path (a
runtime tuple/record cannot build without the value-heap imports), so `compile_module` either
dead-stubs the function (when `main` structurally projected a scalar out of it and never calls it at
runtime) or RETRIES in runtime mode where the imports exist. The `tuple.N` projection then recovers the
scalar at the projection site via the operand's structural shape. Same decline-don't-miscompile gate
the sum constructor already had; the ctor's was just too narrow.
See [[runtime-compound-ctor-declines-unconditionally-on-scalar-path]].
Learning: `spec/learnings/2026-07-07-runtime-tuple-projection-needs-a-let-and-the-direct-path-miscompiles.md`.

**Update (2026-07-07) — 🟢 FIXED.** `tuple.N` now recovers the operand's shape at the projection site
regardless of binding form, so the `let`-free named-def case compiles: `(tuple.0 (dec 4)) → 40`, and
thoroughly (tuple.1 → 5, consumed → 140, compound element matched → 7). The corpus case withheld last
cycle (it FAILed the gate as an invalid component) now lands **green**: `05-compound-types.sexp` *"a
scalar element is projected directly from a function's runtime tuple result"*. Clean lifecycle for a
decline-don't-miscompile violation: invalid → withheld → fixed → pinned green. ⚠ SEED-GAPS Tier 2e still
carries a stale "still produces a VALID component that TRAPS" note — the seed runs it correctly now; the
handoff lags. Consolidated in
`spec/learnings/2026-07-07-the-invalid-component-violation-fixed-and-the-handoff-lags-the-seed.md`.

---
