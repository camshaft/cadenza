# FOLLOW-UP (v-compiler-ml, self): two narrow-annotation over-decline gaps vs reference

Found 2026-07-20 (trunk e873d0806) in a composition conformance sweep. Both are DECLINES where the reference
RUNS — real over-declines in my lane, both non-trivial.

## Gap A: a narrow annotation `+` a COMPUTED (non-literal) operand declines — item-3 HM territory
```
(do (def (main) (+ (: 100 Int8) (+ 10 10))) (export main))       ml=declined   ref=120
(do (def (main) (+ (: 100 Int8) (if (< 1 2) 20 30))) (export main))  ml=declined   ref=120
```
BUT the plain-literal case ADAPTS: `(+ (: 100 Int8) 20)` → 120 (ml). So `arith-lit-adapt` (infer-db) only grounds
a BARE `NLit` sibling to the narrow width; a COMPUTED Int64 subexpression (`(+ 10 10)`, an `if`-expr) is not
adapted → the mixed-width gate rejects. rcdzc unifies the WHOLE unification-connected component (the `(+ 10 10)`
grounds to Int8 through the `+`). This is the KNOWN item-3 HM literal-width-unification gap (see
compiler-ml-foundation-hardening-sequence: "a bare literal's width flows from ANYWHERE in its
unification-connected component, incl. arith operands + if-join arms"). The fix is real HM unification of int
types (deferred-width grounds through arith/if-join), NOT a widened NLit-only adapter. LARGE (item-3), touches
infer-db's `Typed` (needs a deferred int state) + a unify step. Deferred to the item-3 HM slice.

## Gap B: MIXED typed/untyped 2-param defs decline — tractable reader extension
```
(do (def (f (: a Int8) b)      (+ a b)) (def (main) (f 100 20)) (export main))   ml=declined   ref=120
(do (def (f a (: b Int8))      (+ a b)) (def (main) (f 100 20)) (export main))   ml=declined   ref=120
(do (def (f (: a Int8) (: b Int8)) (+ a b)) (def (main) (f 100 20)) (export main))  ml=declined  ref=120
```
`sread.read-def-body`: a `(` first param → `read-typed-param`, which handles a SINGLE typed param
(param2Id = -1) and does NOT read a second param. And `read-2nd-param-or-close` (the untyped-first path) only
handles a second UNTYPED atom, not a `(: b T)`. So ANY 2-param def with a typed param declines. The 2-param
UNtyped case works (`add a b`); the SINGLE typed-param case works (`(f (: a Int8))`). Fix: extend the param
readers so a typed param composes with a second param (typed or untyped) — read-typed-param should, after the
first typed param, look for a second param (mirroring read-2nd-param-or-close) rather than closing at param2 -1;
and read-2nd-param-or-close should accept a `(: b T)` second param. Threads the param-type table for both.
Medium slice (reader + the param-type recording), MY lane (sread + parse-db param tables). Gate: run-src @tests
+ reference-checked. Verify (f 100 20)→120 for all 3 mixed forms; single-typed + untyped-2-param still work.

## Why HELD
A 2-param-if coverage MR (9e43bbae6) was pending → couldn't sync to a clean base, and neither gap is a quick
fix (A is item-3 HM, B is a reader extension worth its own gated slice). Pick up on clean trunk. Gap B is the
better near-term slice (self-contained reader work); Gap A folds into the item-3 HM effort.
