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

## UPDATE 2026-07-20 (Gap A confirmed = item-3 HM, NOT a small arith-lit-adapt extension)
Confirmed by reading arith-result-type/arith-lit-adapt: a MIXED typed/untyped PARAM body-arith declines —
`(def (f a (: b Int8)) (+ a b))` `(f 5 20)` → declined, ref=25 (and the (: a Int8) b mirror). MECHANISM:
arith-result-type unifies operand int-types; on a mismatch it calls arith-lit-adapt, which ONLY adapts a bare
NLit operand to a narrow sibling. An untyped PARAM is an NVar typed Int64 (via param-bind-type from its
Int64-literal arg), NOT an NLit → arith-lit-adapt doesn't touch it → the Int8+Int64 mix declines. The reference
grounds the untyped param to Int8 through the whole unification-connected component. That requires a DEFERRED
int-type in `Typed` + real unification (deferred grounds to a concrete narrow sibling) — this IS the item-3 HM
slice (see compiler-ml-foundation-hardening-sequence's "STEP 1..5" recipe: add TIntW width-0=deferred, replace
both-int64 with unify-int, root-default ungrounded to Int64). NOT a quick arith-lit-adapt tweak. So Gap A =
item-3 HM; do it as that dedicated slice. Same root as the computed-operand case (`(+ (: 100 Int8) (+ 10 10))`).

## UPDATE 2026-07-21: Gap B (mixed typed/untyped param PARSING + typed-param-narrow-bind) is DONE — landed.
Gap B's PARSING half + the typed-param2/param3 narrow-bind are LANDED this session (dfa93bf55 2-param typed,
b22cff6bc typed-3rd): the reader now parses ANY typed/untyped param mix (1/2/3 params), and infer binds each
typed param to its declared narrow type + fit-checks its arg (CDZ0302). So `(def (f (: a Int8) (: b Int8)) …)`,
`(f (: a Int8) b)`, `(f a (: b Int8))`, and all-typed 3-param RUN. What REMAINS from Gap B is ONLY the
mixed-width BODY-ARITH case `(def (f a (: b Int8)) (+ a b))` — that's the untyped-param-adapts-to-narrow-sibling
unification = Gap A = item-3 HM (see the Gap A update above). So Gap B is effectively CLOSED except for the
item-3-HM overlap. Net remaining in this file = item-3 HM (deferred-int-type + real unify), which subsumes
both Gap A and the Gap-B body-arith remainder. Everything else here is landed.

## UPDATE 2026-07-21 (v-compiler-ml): the net-remaining item (item-3 HM) is IMPLEMENTED + SENT (pending land)
This file's net-remaining scope was "item-3 HM (deferred-int + real unify), which subsumes both Gap A and the
Gap-B body-arith remainder". That is now IMPLEMENTED and SENT to pr-sync (ref 99d5dc57e, "item-3 HM — deferred-int
literals ground to a narrow sibling"). Verified vs rcdzc: narrow+computed=120, narrow+if=120, mixed-param-body
`(def (f a (: b Int8)) (+ a b))`=25, nested-inner-wide=100, narrow+lit-fits=120 all RUN; narrow+lit-overflow,
bare-lit-out-of-range (CDZ0201), real mixed-width (CDZ0301) all DECLINE. NOT YET on trunk (MR queued). Will mark
this file RESOLVED once it lands. Design + mechanism: vcml-design-item3-hm-deferred-int-infer-boundary.

## RESOLVED 2026-07-21 (v-compiler-ml): item-3 HM LANDED on trunk (c2116abf5, integrate MR 99d5dc57e)
The net-remaining scope (item-3 HM = deferred-int grounds to a narrow sibling, subsuming Gap A + the Gap-B
body-arith remainder) is now ON TRUNK. Gap A cases RUN: narrow+computed, narrow+if, mixed typed/untyped
param-body `(def (f a (: b Int8)) (+ a b))`, nested-inner-wide; guards still DECLINE (bare-lit out-of-range
CDZ0201, real mixed-width CDZ0301). Nothing open here — closing.
