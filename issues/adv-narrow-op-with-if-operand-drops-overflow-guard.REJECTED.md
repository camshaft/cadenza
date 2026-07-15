# REJECTED (false positive) — `adv-narrow-op-with-if-operand-drops-overflow-guard`

**Verdict:** NOT A BUG. Adjudicated twice (iter-181 filing, iter-189 re-file) against the
language spec. The compiler behavior is correct; the finding's premise is wrong.
Resolved on `spec` by commit `d73066de` (three positive regression cases pinned in
`spec/semantics/06-numeric-model.sexp`). **Do not re-file — read this first.**

## The claim

`(def (main (: n Int8)) (+ (if (< n 5) 100 0) 100))` with n=3 runs to 200 and should
instead `(trap "integer overflow")` because 200 doesn't fit Int8. The re-file (iter-189)
added a control-flow-free witness `(+ (+ 50 50) 50)` → 150, and `(+ (if (< n 5) 50 0) 100)`
→ 150.

## Why it is wrong

In every witness the arithmetic operands are **deferred-width integer literals with no
constraint**, so per `spec/capabilities/numeric-model.md` §"Default Literal Type" —
*"an integer literal with no other constraint MUST take the numeric model's default
integer type"* (Int64) — the `+`/`-`/`*` node types as **Int64**. 200 and 150 are
representable, correct Int64 results. There is no Int8 arithmetic to overflow.

The `Int8` param `n` appears **only in the condition** `(< n 5)` — a condition is not a
width constraint on the enclosing arithmetic op. In the iter-189 nested-arith witness
`(+ (+ 50 50) 50)`, `n` is **completely unused**: the program is byte-for-byte equivalent
to the no-param `(def (main) (+ (+ 50 50) 50))`, which also types Int64 → 150. The Int8
param is provably irrelevant to the result.

Verified with `type_of` on the op node in all witnesses (if/let/nested-arith, +/-/*,
Int8/Int16/UInt8): **the op node types Int64 in every case.**

## The wrap-down guard is NOT dropped (the finding's root-cause hypothesis is disproven)

When the op is **genuinely narrow** — because something actually constrains it — the
`@c09a15ef`/`@98db6bb0` i64→i32 wrap-down of a deferred-width operand **keeps** its
overflow range-check:

- return annotation `(: (+ (if (< n 5) 100 0) 100) Int8)` n=3 → **traps** (overflow);
  n=9 (in-range) → exact **100**.
- annotated operand `(+ (: 100 Int8) (if (< n 5) 100 0))` → **traps**.
- annotated branch `(+ (if (< n 5) (: 100 Int8) (: 0 Int8)) 100)` → **traps**.
- return-annotated nested const arith `(: (+ (+ 50 50) 50) Int8)` → **CDZ0302** at
  compile time (the const-fold catches the overflow — the guard the finding wants,
  already present).

The finding's own "explicitly-Int8 branches RESTORE the guard" evidence is exactly this:
that isn't a bug trigger, it's the op being Int8 instead of Int64.

## Correct behavior, pinned as regression cases on spec

`06-numeric-model.sexp` now contains, next to the narrow-negation runtime cases:
1. an if-operand op with deferred-width branches is Int64 → **200** (correct value);
2. a genuinely-narrow (return-annotated) op with a wrapped-down if-operand **traps** on
   overflow (guard survives the wrap-down);
3. the in-range companion → exact **100** (guard does not over-fire).

## For the producer / future fires

The producer re-files because it grades the runtime symptom (200 ≠ trap) without checking
the op's inferred type. Before treating "a narrow op drops its guard" as a FAIL, check
`type_of` of the **op node**: a narrow param in a *condition* (or an unused param) is not a
width constraint on the arithmetic. If the op types Int64, the wide result is correct.
