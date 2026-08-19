# toll-hole-handle — nested closed handle in the TOLL position (SILENT MISCOMPILE)

## Finding: pyth1 — nested closed handle in the post-resume TOLL position miscompiles
```
(handle E (% n 3)
  ((tick () s
    (+ (resume (+ s 1) (* 10 s))               ; two-hole (non-tail) arm
       (handle E 40 ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2)))))  ; TOLL = closed handle = 42
  (+ (E.tick) (* 10 (E.tick))))
```
The additive term beside `resume` (the per-dispatch "toll") is a nested CLOSED handle
that reduces to 42. Deep handler, outer body has two E.tick dispatches.

## Verdict: SILENT MISCOMPILE (wrong value, NOT a decline) — uniform wasm+rust+rust-async
- **Compiler**: value **1414** (n=10), **1312** (n=0) — compiled and RAN, wrong answer.
- **Correct**: 196 / 95. Established THREE independent ways:
  1. Closed-form hand model → 196/95.
  2. Independent operational simulation (python generator CPS deep-handler) → 196/95.
  3. **Referential-transparency control** (`pyth1-ctrl.sexp`, literal `42` in the toll):
     PASSES 196/95 on all three backends.
- **Inner handle standalone** (`/tmp/inner-standalone.sexp`): correctly = 42.
- **Distinct-effect differential** (`pyth1-distinct.sexp`, inner over fresh F):
  IDENTICAL wrong 1414/1312 → NOT a routing/shadowing leak; a genuine VALUE bug.

## Why this is decisive (pyre3 lesson applied)
Uniform ×3 + distinct-effect-identical proves ROUTING-INDEPENDENCE, not correctness.
The referential-transparency control is the proof: substituting the closed handle's
literal value (42) gives 196/95, but the nested handle giving 42 gives 1414/1312.
That violates referential transparency → miscompile.

## Lead for v-effects
Wrong-minus-correct is a near-CONSTANT offset regardless of seed:
  n=10: 1414-196 = 1218 ;  n=0: 1312-95 = 1217.
A fixed extra contribution independent of the outer seed is consistent with the inner
(closed) handle's body `(+ (E.tick) 2)` being mis-lowered — its E.tick dispatches
appear to leak extra work into the fold rather than the inner handle folding to a
self-contained 42. Contrast pyre6 (SAME nested handle, but in the ANSWER hole) which
compiles CORRECTLY — the TOLL position (post-resume additive term) is the one that
miscompiles.

## pyth2 — DISCRIMINATOR: non-dispatching nested handle in the toll (tick 1865)
Same shape but the inner nested handle's body performs NOTHING (constant body =7):
`(handle E 40 ((tick () t (resume t (+ t 1)))) (: 7 Int64))` in the toll.
**PASSES 126/25 on all three backends.** So *installing* a handle in the toll is fine;
the pyth1 miscompile is triggered specifically when the inner handle **DISPATCHES** its
own effect inside the toll (pyth1's `(+ (E.tick) 2)` body). Isolates the bug to
dispatching-nested-handle × toll-position. pyth2 is itself a valid PASS-WITNESS
(promotable once the pyth1 neighborhood ruling settles).

## Status: RULED MISCOMPILE + FIX LANDED (7bc8916f9)
v-effects RULED pyth1 a MISCOMPILE (196/95 correct); fix LANDED on origin/main as
**7bc8916f9** ("decline a nested-handle post-resume TOLL in the two-hole refold").
Post-fix build (verified tick 1866):
- pyth1 + pyth1-distinct now cleanly DECLINE (silent 1414 GONE).
- pyth1-ctrl (literal 42 toll) stays PASS 196/95.
- **pyth2 (non-dispatching inner handle in toll) now ALSO DECLINES** — it was a PASS
  pre-fix. The fix declines ANY nested handle in the toll position (broader than the
  dispatching-only cut, but SAFE: reject-not-miscompile). So pyth2 is no longer a
  pass-witness; it is a decline-witness alongside pyth1.
All held as decline/todo-witnesses (NO baseline row); oracles at ruled-correct values
(196/95, 126/25) so they auto-flip to pass when the durable correct-FOLD lands (which
covers next-state pyre3/4/5, seed pyse1, AND toll pyth1 together). This is the 5th
probe-driven fix to reach trunk (pyr3, pyr7, pyre3-decline, pyth1-decline + pyg1 ICE
deferred).
