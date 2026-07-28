# DESIGN (v-compiler-ml, self): item-3 HM (Gap A) — the deferred-int MACHINERY ALREADY EXISTS; the gap is one infer-db boundary

Scoped 2026-07-21 (base d00a9027d, param4-wiring resend e24228335 pending at pr-sync). Gap A = a narrow-typed
operand `+` a COMPUTED / if-expr / untyped-param operand DECLINES where the reference RUNS (grounds the whole
unification-connected component to the narrow width). Confirmed by probe:
```
(do (def (main) (+ (: 100 Int8) 20)) (export main))            ml=120       (bare-lit adapts — works)
(do (def (main) (+ (: 100 Int8) (+ 10 10))) (export main))     ml=declined  ref=120  (computed operand)
(do (def (main) (+ (: 100 Int8) (if (< 1 2) 20 30))) …)        ml=declined  ref=120  (if-expr operand)
(do (def (f a (: b Int8)) (+ a b)) (def (main) (f 5 20)) …)    ml=declined  ref=25   (untyped param operand)
```

## KEY FINDING: the hard part is DONE. The `ty`/`unify` layer already has full deferred-int support.
Reading ty.cdz + unify.cdz + ty-bridge.cdz:
- `ty.cdz`: `ty-deferred-int()` = `TyInt(IntType(SignDef, WidthDef))`; `ty-fixed-int(s,w)`; `ground-default(t)`
  grounds SignDef→Signed + WidthDef→WFixed(64). The `Sign`/`Width` types already carry a DEFERRED state.
- `unify.cdz`/`unify-ty.cdz`: `unify-int` ALREADY unifies deferred↔concrete BOTH directions + deferred↔deferred
  (tested: unify-intw-deferred-grounds-left/right/meets-deferred). A deferred int takes its sibling's sign+width.
- `ty-bridge.cdz`: `ty-to-typed` GROUNDS any deferred `Ty` back to a concrete pipeline `Typed` (via
  ground-default) — so the pipeline (lower/eval/emit) NEVER sees a deferred type. Invariant already documented +
  tested (tb-deferred-ty-grounds-back).

So the "STEP 1..5 add-a-deferred-state-everywhere" recipe in compiler-ml-foundation-hardening-sequence is STALE /
over-scoped — that state EXISTS. lower-db (7 Typed sites) + ty-bridge (30) + emit/eval need NO change (they only
ever see grounded types, guaranteed by ty-to-typed).

## THE ACTUAL GAP: infer-db `arith-result-type` fixes both operands before unifying
`arith-result-type` (infer-db.cdz:294) does `unify-ty(ty-fixed-int(sa,wa), ty-fixed-int(sb,wb))` — it converts
BOTH operands to FIXED Ty ints, so a defaulted-Int64 operand (a computed subexpr, an if-join, an untyped param —
all typed `TIntW(signed,64)` by default) is FIXED at width 64, not DEFERRED. Unify then sees Int8~Int64 = fixed
width mismatch → fails → falls to `arith-lit-adapt`, which ONLY rescues a bare `NLit` (via node-at = NLit). A
computed/if/param operand isn't an NLit → no rescue → declines.

The reference defers a "this is a defaulted Int64, not an explicitly-Int64 operand" operand so unify grounds it
to the narrow sibling. i.e. an operand whose width came from the DEFAULT (a literal or a literal-derived
computation), NOT from an explicit `(: e Int64)` annotation, should map to `ty-deferred-int()` at the infer↔ty
boundary — then the EXISTING unify machinery does the rest, and ty-to-typed grounds the result.

## THE HARD SUB-PROBLEM (why this isn't a 1-liner): "defaulted" vs "explicitly Int64" is not tracked
Today `Typed.TIntW(signed, 64)` is AMBIGUOUS: it's produced BOTH by a bare literal / computed-from-literals
(should be deferrable) AND by an explicit `(: x Int64)` annotation or a wide runtime value (must NOT silently
re-width to Int8). To defer correctly, infer must DISTINGUISH them. Options:
- **(A) Add a deferred state to `Typed`** (mirror the Ty layer): `Typed.TIntDeferred` or a flag on TIntW. Then a
  literal/computed-from-deferred produces the deferred Typed; an annotation produces fixed. Ripples through
  infer-db's 76 Typed sites (many are trivial passthrough) — but lower/ty-bridge unaffected (ground before they
  see it). This is the FAITHFUL fix (matches how Ty already works) and is the recommended one. MEDIUM slice,
  ALL in infer-db + ty-bridge's typed-to-ty (map deferred Typed → ty-deferred-int).
- **(B) Propagate deferral structurally at arith-result-type**: instead of a new Typed state, make
  arith-result-type recurse — an operand that is itself an arith/if node whose operands are deferrable is
  deferrable. Fragile + duplicates the unify logic; NOT recommended.

RECOMMEND (A). The Typed deferred-state is the missing mirror of the Ty deferred-state that already exists.

## Landing plan (single gated slice, infer-db-local)
1. Add `Typed.TIntDeferred` (or `TIntW`+a `deferred:Bool`). t-int64() stays the GROUNDED default for
   annotation/runtime; add `t-int-deferred()` for a bare literal + a computed-from-deferred result.
2. NLit → t-int-deferred() (was t-int64). An annotated `(: e T)` → fixed TIntW. An untyped PARAM bound from a
   defaulted-Int64 arg → deferred (so `(def (f a (: b Int8)) (+ a b))` grounds a to Int8).
3. arith-result-type: map a deferred Typed operand → `ty-deferred-int()`, a fixed one → `ty-fixed-int`; let the
   EXISTING unify do the grounding; ty-to-typed grounds the result. DELETE arith-lit-adapt + lit-fits-narrow
   (subsumed — the deferred literal now grounds through unify, incl. the fits-check which unify-int enforces).
4. ty-bridge typed-to-ty: deferred Typed → ty-deferred-int().
5. Gate: the 4 Gap-A probes above → all RUN at the narrow width; PLUS a real mixed-width `(: 5 Int8) + (: 5
   Int16)` STILL declines (fixed≁fixed); overflow of the grounded narrow result still traps (CDZ0304); all
   existing infer/conformance tests green. Verify each vs rcdzc.

## Why not now
Base pinned: my param4-wiring resend (e24228335) is queued at pr-sync; sync REFUSES to rebase (would orphan the
--ref). This slice is IN infer-db — the same file — so it cannot be stacked either. Execute on clean trunk once
the param4 MR lands (`cargo xtask sync --force`). This is the RECOMMENDED next big slice AFTER param4 lands
(smaller than forward-ref now that the deferred machinery is confirmed to exist, and independent of it).
See also vcml-followup-narrow-annotation-over-decline-gaps (the original Gap-A/B report) + pr584/pr687 (prior
deferred-width sign/literal work — the Ty-layer machinery this builds on).

## VERIFIED (2026-07-21, same tick): the boundary wiring the fix depends on — machinery green, exact touch-points
Ran the ty/unify test files on base d00a9027d: **unify.cdz 21/0** (incl unify-intw-deferred-grounds-left/right/
meets-deferred/sign-checked-under-deferred-width) + **ty.cdz 14/0** (ty-deferred-is-int, ty-ground-default-
deferred-to-int64, partial-deferred-sign). The deferred machinery is real + green. Traced the exact boundary fns:
- `ty-to-typed-int` (infer-db:283) AND `ty-bridge.ty-to-typed` (ty-bridge:28) BOTH call `ground-default(t)` FIRST
  → a deferred result of `unify(deferred, Int8)` grounds to Int8 automatically. RESULT side needs NO change. ✅
- `arith-result-type` (infer-db:299) builds `ty-fixed-int(sa,wa)`/`ty-fixed-int(sb,wb)` INLINE (not via
  typed-to-ty) → THIS is the one site to make deferred-aware: a deferred Typed operand → `ty-deferred-int()`.
- `ty-bridge.typed-to-ty` (ty-bridge:21) maps `TIntW(s,w) → ty-fixed-int(s,w)` ALWAYS fixed → needs a new arm
  `TIntDeferred → ty-deferred-int()` (used anywhere infer→ty crosses outside arith-result-type).
So the mechanical touch-list is: (1) add `Typed.TIntDeferred` + `t-int-deferred()`; (2) NLit → deferred (was
t-int64); (3) untyped-param-bound-from-defaulted-arg → deferred; (4) arith-result-type: deferred operand →
ty-deferred-int, else ty-fixed-int (the result stays deferred if both deferred → grounds at root via
ty-to-typed-int); (5) typed-to-ty: +TIntDeferred arm; (6) delete arith-lit-adapt/lit-fits-narrow. The result-side
grounding (ty-to-typed*) is already correct. Confirms the slice is infer-db + ty-bridge-local, MEDIUM, no
lower/emit/eval change.

## REGRESSION-GUARD BASELINE (2026-07-21, base 01ad6b9b6-era) — the item-3 HM change MUST preserve these
Probed current behavior; the deferred-int slice (NLit→deferred + delete arith-lit-adapt) must keep ALL of these:
```
(+ (: 100 Int8) 20)                        → 120        (deferred lit grounds to Int8, fits)
(+ (: 100 Int8) 100)                       → declined   (grounded Int8 result 200 overflows → CDZ0304 trap)
(def (f (: a Int8)) a) (f 200)             → declined   (arg 200 out of Int8 range → CDZ0302)
(+ (: 5 Int8) (: 5 Int16))                 → declined   (CDZ0301 — TWO FIXED narrows ≁, MUST NOT ground)  ← KEY
(+ 1000000 2000000)                        → 3000000    (plain Int64, both deferred → ground to default i64)
```
The KEY safety property: two EXPLICITLY-annotated narrow types (fixed↛fixed) must STILL fail to unify — only a
DEFERRED (literal-derived) operand grounds to a sibling. The design preserves this because an annotation `(: e T)`
produces a FIXED TIntW (never deferred), so `(: 5 Int8) + (: 5 Int16)` stays fixed≁fixed → declines. Overflow of a
GROUNDED narrow result still traps (unify-int's fits-check + eval's narrow-overflow trap are unchanged). Add these
5 as explicit conformance @tests in the slice.

## WHY a Typed change is UNAVOIDABLE (2026-07-21 scoping refinement) — rules out the cheap alternatives
Considered two cheaper approaches to avoid touching `Typed` (which has 45 exhaustive matches in infer-db +
sites in lower-db/ty-bridge):
1. **Width-0 sentinel on TIntW** (`TIntW(s,0)` = deferred): REJECTED — a bare literal is BOTH sign- AND
   width-polymorphic (existing arith-lit-adapt grounds a literal to a sibling's full (sign,width), incl. adapting
   a signed-default literal to an UNSIGNED narrow sibling). A width-only sentinel loses sign-deferral → would
   regress the unsigned-adapt case. Would need a sign sentinel too, making TIntW's fields overloaded + every
   width/sign inspection leaky.
2. **Detect a bare NLit operand IN arith-result-type** (it has the node ids a,b): REJECTED — this only
   re-implements what arith-lit-adapt ALREADY does (the bare-NLit case works today). The REAL gap is the
   COMPUTED / if-expr / untyped-param operand, which is NOT an NLit. By the time arith-result-type sees operand
   `a`, its type is already `TIntW(signed,64)` with NO memory of whether it flowed from literals (deferrable) or
   an explicit `(: e Int64)` / runtime value (fixed). The deferred-ness MUST propagate THROUGH NBin/NIf/NVar
   typing → the TYPE must carry it → a Typed change is unavoidable.
CONCLUSION: option (A) — add the deferred state to `Typed` (mirror the Ty layer's SignDef+WidthDef) — is
confirmed the right + only faithful fix. It IS a medium slice (the 45 matches are mostly trivial passthrough that
can share a helper, but each must compile). Route a deferred Typed → ty-deferred-int() at arith-result-type;
ground-default + ty-to-typed-int already collapse it back before lower/ty-bridge. Do it as ONE focused slice on a
clean base, not stacked. NLit → deferred; annotation/param-declared-narrow → fixed; untyped-param-from-defaulted-
arg → deferred.

## 🩸 CRITICAL CORRECTION (2026-07-21, clean-trunk 9b3a0f47a, pre-implementation audit): unify does NOT value-check → the slice is BIGGER than "delete arith-lit-adapt"
Auditing before implementing, found the design's "delete arith-lit-adapt, let unify ground the literal" plan is
UNSOUND as stated. `unify-int` (unify-ty.cdz:40) unifies PURELY at the type level (sign+width axes via
unify-sign/unify-width); it does NOT check the literal's VALUE fits, and the deferred `Ty` carries no value. So
routing a deferred literal through pure unify would make `(: 100 Int8) + 200` unify Int8~deferred → Int8 and
WRONGLY ACCEPT 200 — a LAUNDERED-OVERFLOW MISCOMPILE (the current arith-lit-adapt→lit-fits-narrow→fits-width
value-gate is exactly what PREVENTS this; test id-narrow-plus-non-fitting-literal-still-err pins it).

IMPLICATIONS:
1. The deferred-int slice MUST RETAIN a value-fits gate when grounding a deferred operand to a narrow width — it
   canNOT just be unify. Keep lit-fits-narrow's fits-width check (or fold an equivalent into the grounding step).
2. HARDER: a COMPUTED operand (`(+ 10 10)`, an if-join) has NO single literal value to fit-check — the reference
   must fit-check the COMPUTED RESULT against the narrow width. That's a value-flow the current infer (pure type
   column) doesn't do. rcdzc likely grounds the type via unification AND relies on the eval/lower NARROW-OVERFLOW
   TRAP (eval-db:34 `overflows(v,signed,width)`) to catch a runtime overflow of the grounded narrow result —
   i.e. `(+ (: 100 Int8) (+ 10 10))` grounds the whole thing to Int8, computes 120 (fits) → OK; if it were
   `(+ (: 100 Int8) (+ 100 100))` it grounds to Int8, computes 200 → the EVAL TRAP fires → declines. So the
   value-safety for COMPUTED operands comes FREE from the existing narrow-overflow trap ONCE the type grounds
   narrow — NO compile-time value-check needed for computed operands (only for a BARE literal, where there's no
   arith op to trap, arith-lit-adapt's compile-time fits-check still applies... but actually a bare literal IS an
   operand of the enclosing arith op, so its overflow would ALSO trap at eval — need to VERIFY whether the
   compile-time CDZ0302/CDZ0304 distinction matters vs a runtime trap).
3. THEREFORE re-verify against rcdzc: does `(: 100 Int8) + 200` DECLINE at compile (CDZ0302, current ml behavior)
   or TRAP at runtime? And `(+ (: 100 Int8) (+ 100 100))`? The answer determines whether the fits-gate is
   compile-time (keep lit-fits-narrow) or can defer to the eval trap. This is the KEY question to settle (probe
   rcdzc) BEFORE implementing — it changes whether arith-lit-adapt is deleted or kept-and-extended.

STATUS: item-3 HM is NOT a mechanical slice — it has a live correctness fork (compile-fits vs runtime-trap) that
must be resolved against rcdzc first. Do that probe as the FIRST step of the implementation tick. Downgraded
from "fully de-risked" to "de-risked EXCEPT the fits-check semantics, which needs one rcdzc probe."

## ✅ FORK RESOLVED (2026-07-21, probed rcdzc directly) — the ML slice is SIMPLER than the reference's machinery
Probed rcdzc (`cdz compile` on s-expr):
- `(+ (: 100 Int8) 200)` → CDZ0304 (compile-provable overflow) + CDZ0201 (200 out of Int8 range) — DECLINES.
- `(+ (: 100 Int8) (+ 10 10))` [=120, fits] → COMPILES (grounds whole thing to Int8).
- `(+ (: 100 Int8) (+ 100 100))` [=200, overflows] → CDZ0304 — DECLINES.
- `(def (f a (: b Int8)) (+ a b))` `(f 5 20)` → COMPILES.
So the reference grounds the connected component to the narrow width, then const-folds + compile-overflow-checks.

KEY: the ML compiler has NO compile-time const-fold; it handles narrow overflow via the EVAL TRAP (eval-db:34
`overflows(v,signed,width)` → None → "declined"). For the DIFFERENTIAL gate, ml's eval-trap-decline and rcdzc's
CDZ0304-compile-decline are the SAME observable outcome (both decline). So the ML slice does NOT need
const-fold: just GROUND the deferred literal-derived operand to the narrow sibling (type becomes Int8) so the
CBin carries width 8 → the existing eval narrow-overflow trap catches any overflow of the grounded result.

THE MINIMAL CORRECT ML SLICE:
1. Type a bare NLit as DEFERRED (sentinel `TIntW(_,0)` or a variant). Propagate deferred-ness through NBin/NIf so
   a computed-from-deferred subexpr stays deferred (an all-literal `(+ 10 10)` is deferred; a subexpr containing
   an annotated/param operand is NOT).
2. arith-result-type: a deferred operand unifies with a fixed narrow sibling → the NARROW type (ground via
   ty-deferred-int + unify → ty-to-typed-int). Two deferreds → stay deferred (ground to Int64 at root). The CBin
   result carries the grounded (narrow) width → lower emits CBin(op, signed, width=8,…) → eval traps overflow.
3. KEEP a compile-time fits-check ONLY where there's no enclosing arith op to trap (a bare narrow-annotated
   literal alone) — but that's already NAnnLit-handled. For arith, the eval trap suffices → arith-lit-adapt CAN
   be replaced by proper deferral (its value-check is subsumed by the eval trap on the grounded narrow result).
   VERIFY: `(: 100 Int8) + 200` must still decline — with 200 as a deferred literal grounding to Int8, the CBin
   is Int8, eval computes 100+200=300 wait no — 200 alone doesn't fit Int8 as an operand VALUE. NEED: does eval
   check each OPERAND fits its narrow type, or only the RESULT? eval-db:34 checks the CBIN RESULT. A 200 operand
   typed Int8 but never range-checked as an operand would be a hole. → so KEEP lit-fits-narrow's per-literal
   compile fits-check for the bare-literal-operand case; ADD deferral for the computed/param case (which the
   eval result-trap covers). Net: EXTEND, don't delete, arith-lit-adapt.

STATUS: fully resolved semantics. Slice = add deferred-int typing + propagate + extend arith-result-type; keep
the bare-literal compile fits-check; computed/param overflow rides the eval trap. Medium, infer-db-local. Ready
to implement as one gated slice.
