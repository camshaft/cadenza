# foreign-draw-in-arm-body — QUESTION-WITNESS (model-risk, NOT a filed miscompile)

## pyfb1 — A's arm performs a distinct counter effect B mid-body, uses it in resume
```
(handle B 0 ((beat () bs (resume (+ bs 1) (+ bs 1))))    ; B counter
  (handle A (% n 3)
    ((tick () s (let ((k (B.beat))) (resume (+ s k) (* s k)))))
    (+ (A.tick) (* 100 (A.tick)))))
```
My model: 302/201. Compiler: 502/201 (n=0 AGREES, n=10 off by 200).

## Status: QUESTION-WITNESS (pysh3-class model-risk) — HELD UNPROMOTED, filed as ASK not finding
Investigation isolated the discrepancy to the NUMBER OF ARM ENTRIES for a two-dispatch
body `(+ (A.tick) (A.tick))`:
- Single A.tick: correct (verified 2/1, B.total=1). No issue.
- Two A.ticks: the arm's B.beat fires **3 times**, not the 2 my model expects — and this is
  UNIFORM across variants (k-in-answer-only, k-in-both-holes, and even a pure control
  `(do (B.beat) (resume (+ s 1) (+ s 1)))` all show B.total=3). So it is NOT about the
  let-bound foreign draw or reusing k across holes — it is the deep-handler re-entry COUNT
  for two sequential performs in `(+ _ _)`.
- Pure next-state threads correctly (state tick2 sees is consistent with the beat count).

Because the compiler is uniform x3 backends AND consistent across all variants, and my
deep-handler continuation-re-entry counting is exactly the pysh3-class thing I have gotten
WRONG before (uniform+consistent historically = my model wrong, not a bug), I am NOT
filing this as a miscompile. Filed to v-effects as a QUESTION: is 3 arm-entries for
`(+ (A.tick) (A.tick))` correct deep-handler semantics (e.g. a continuation re-entry I am
mis-counting), or a genuine extra-perform bug? Oracle left at my model values but UNTRUSTED.

Banked: pyfb1.sexp (observation), pyfb-count-control.sexp (the 3-beat count control).
DO NOT PROMOTE until ruled.

## RULING (v-effects, tick after 1885): GENUINE SILENT MISCOMPILE — correct count = 2, compiler does 3
v-effects RULED pyfb1 a REAL silent miscompile, NOT a model error. Airtight argument
(independent of deep-handler nesting): A's arm body executes EXACTLY ONCE per A.tick
discharged; (+ (A.tick) (A.tick)) = 2 one-shot performs -> 2 arm executions -> 2 B.beats.
The compiler does 3 => one EXTRA foreign perform.
Triangulated (readout returns body_beats+1):
- directBB (+ (B.beat)(B.beat)) single handler -> 2 beats. CORRECT (readout sound).
- single A.tick -> 1 beat. CORRECT.
- (+ (A.tick)(A.tick)) nested A-over-B -> 3 beats. WRONG (my B.total=3).
- let-bound two A.tick -> also 3 (NOT the + form; it's two sequential A.tick under the
  nested handler).
CPS: [[handle_A (+ (A.tick)(A.tick))]] = do B.beat; do B.beat; 5 -> 2 beats.
CLASS: same as xhsC foreign-perform-duplication (foreign perform in the arm re-executed
by the nested-handler fold) but the SIMPLEST shape — no shared-let, no binder. SILENT
(only observable via B's state/count), uniform x3 => genuine value bug.

## Status: MISCOMPILE-WITNESS (v-effects owns the fix, effects-fold lane)
Keep banked as a MISCOMPILE-witness with oracle = CORRECT count 2 (pyfb1's 302/201 model
is the correct answer; the compiler's 502/201 is the bug). v-effects acting per the
silent-miscompile standing instruction (fix, or safe-decline first if deep). This is the
6th probe-driven finding (pyr3, pyr7, pyre3, pyth1, + the question-witness pysh3 ruled
correct, now pyfb1 ruled miscompile). The question-not-bug framing (pysh3 protocol) was
correct process: banked observations only, filed as a question, verified rigorously before
any claim — and it crossed into real, cleanly.

## SCALING (tick after ruling): TRIANGULAR foreign-perform duplication
Characterized how the extra performs scale with N = number of A.tick dispatches in the body:
- N=2 (+ (A.tick)(A.tick)): 3 beats (correct 2), extra +1
- N=3: 6 beats (correct 3), extra +3
- N=4: 10 beats (correct 4), extra +6
beat count = T(N) = N(N+1)/2 EXACTLY (3, 6, 10). The nested-handler fold re-executes the
foreign perform TRIANGULARLY: the k-th A.tick re-runs the foreign performs of ALL arms
1..k, not just its own. (Cousin of the #24 exponential fold-duplication class, but
triangular in this foreign-perform-in-arm shape.) Banked pyfb-3tick-count.sexp,
pyfb-4tick-count.sexp. Reported to v-effects — the T(N) signature pins the fix target:
the continuation being re-walked per dispatch re-executes prior arms' foreign performs.

## pyfb2-discarded — value-DISCARDED witness (requested by v-effects for the fix)
Arm does (do (B.beat) (resume (+ s 1) (+ s 1))) — the foreign perform's VALUE is dropped
by the do, but the effect (increment) must still run EXACTLY ONCE per dispatch, and the
pure A-body value is preserved. Body (+ (A.tick)(A.tick)).
- CORRECT (post-fix, oracle): 2 beats -> 20005/20003 (body 2seed+3 + 10000*2).
- PRE-FIX (observed on trunk): 3 beats -> 30005/30003 (same triangular bug; confirms the
  effect fires-and-duplicates even when the perform's value is discarded — pins that the
  fix's "effectful leading stmt runs once" holds for value-dropped performs too).
Oracle set to the CORRECT post-fix 20005/20003 so it auto-flips todo/fail -> pass when
v-effects' do-peel fix (31c788101) lands. Banked as a fix-witness alongside pyfb1.

## pyfb3-nextstate-binder — the safe-floor DECLINE case (distinct on-land expectation)
Arm: (tick () s (let ((k (B.beat))) (resume (+ s 1) (+ s k)))) — the let-bound EFFECTFUL
foreign draw k is READ BY the NEXT-STATE (not just the answer). v-effects' fix
(31c788101) SAFE-FLOOR DECLINES this bind-once-share case (an effectful def-binder read by
the next-state can't be run-once-and-threaded without the triangular re-execution, so it
rejects rather than folds).
- PRE-FIX (observed on trunk): 30006 (3 beats — the triangular bug is present here too).
- POST-FIX EXPECTATION: cleanly DECLINES (todo), NOT a value. So this is a DECLINE-witness,
  distinct from pyfb1/pyfb2 (which flip to PASS). Do NOT promote as a pass-witness; keep as
  a decline-witness confirming the safe-floor boundary. If the fix instead FOLDS it to the
  correct value (2 beats), that's a bonus (the fold got smarter) — re-derive oracle then.
Banked as the safe-floor boundary witness. WATCH on 31c788101 land: expect decline.

## ON-LAND VERIFICATION of fix 5208ad1f3 (tick 1893): PARTIAL — residual miscompile in pyfb1/pyfb3
Fix 5208ad1f3 landed. Verified on a fresh post-fix build:
- FIXED (value-position + pure-next-state family): pyfb2-discarded PASSES 20005/20003
  (2 beats); pyfb-3tick = 30009 (3 beats); pyfb-4tick = 40014 (4 beats). Regression ladder
  3/6/10 -> 3/4/... wait: these are the (do (B.beat) (resume pure)) shapes where k is NOT
  read by next-state. They now fire N beats. CORRECT.
- RESIDUAL MISCOMPILE (effectful let-bound k READ BY next-state): pyfb1 still runs to 502
  (correct 302) and pyfb3 still 30006 — NEITHER declines NOR folds correctly. The fix
  handles value-position and pure next-state, but when the effectful let-bound k feeds the
  NEXT-STATE ((* s k) / (+ s k)), tick2's state is still mis-threaded (observed tick2 state=3,
  should be 1) => 502. v-effects INTENDED this case to safe-floor DECLINE, but it neither
  declines nor computes correctly — the safe-floor is not firing for the let-bound (vs
  inline) effectful-k-in-next-state.
RE-FILED to v-effects as a residual. pyfb1/pyfb3 STAY miscompile-witnesses (oracle: pyfb1
correct=302/201, pyfb3 correct=... or decline). pyfb2-discarded/3tick/4tick are now
PROMOTABLE pass-witnesses.

## pyfb3-valonly — the let-peel VALUE-position case that FOLDS (boundary vs pyfb3)
Arm: (tick () s (let ((k (B.beat))) (resume (+ s k) (+ s 1)))) — effectful let-bound k read
ONLY by the resume VALUE, next-state PURE. FOLDS 402/301 on wasm+rust+rust-async (post-fix
trunk 5208ad1f3; k runs once/dispatch). CONTRAST pyfb3 (k in next-state) still miscompiles
30006. So the let-peel effectful-init boundary is: k-in-VALUE folds, k-in-NEXT-STATE
miscompiles (v-effects' pending ctx-aware foreign-perform fix will make the latter DECLINE).
pyfb3-valonly = PASS-witness pinning the folding side. v-effects confirmed pyfb3-valonly
WOULD fold under their (reverted broad) change; it already folds on trunk today.
Note: v-effects CORRECTION — pyfb3 goes through the LET-peel (twin of pyfb1's do-peel),
a SEPARATE open miscompile the landed 5208ad1f3 did NOT touch; keep pyfb3 a MISCOMPILE-
witness (not decline). Their broad let-peel decline over-declined 24 valid cases
(discharged-op/recursive let-inits) → reverted; correct fix threads HandlerCtx + gates on
FOREIGN perform (op not in ctx.arms).

## pyfb4-single — the SINGLE-dispatch distinguisher (k-in-next-state FOLDS at 1 dispatch)
v-effects' as7 insight: the effectful-let-bound-k-read-by-next-state shape (pyfb3, which
MISCOMPILES at multi-dispatch) FOLDS correctly at a SINGLE dispatch — there's no second
dispatch to re-run the let through the threaded next-state. pyfb4-single:
  (tick () s (let ((k (B.beat))) (resume (+ s 1) (+ s k))))   ; body (+ (A.tick) 100) = 1 dispatch
FOLDS 102/101 on wasm+rust+rust-async. So the DISTINGUISHER for the pyfb3 bug is
MULTI-dispatch (the triangular re-run), not the arm shape itself. This constrains
v-effects' eventual freeze-once fold fix: single-dispatch must STAY folding (pyfb4-single
102/101, as7=6) while multi-dispatch (pyfb3) gets frozen-once. PASS-witness pinning the
single-dispatch floor. v-effects reverted BOTH broad peel-decline attempts (over-declined
single-dispatch as7 + discharged-op cases); correct fix = freeze-once fold gated on
multi-dispatch, not a peel-level decline.

## pyfb5-inline-nextstate — INLINE next-state foreign perform DECLINES (as2 guard), vs pyfb3 let-bound miscompiles
v-effects scope refinement: (resume (+ s 1) (+ s (B.beat))) — foreign perform INLINE directly
in the tail-arm threaded next-state, multi-dispatch — DECLINES cleanly (the existing as1/as2
safe-decline guard for a foreign perform directly in the next-state). Confirmed: declines
"not yet reducible by the tail-resumptive fold". So the INLINE form is ALREADY safe.
pyfb3's miscompile is specifically the LET-BOUND form (let ((k (B.beat))) (resume (+ s 1)
(+ s k))): the perform is hoisted to a let-init and only the BINDER k appears in the
next-state, so the as2 direct-perform guard doesn't fire → it slips through and miscompiles.
So the narrowed bug locus is: LET-HOISTED foreign perform whose BINDER is read by the
next-state under multi-dispatch. pyfb5-inline banked as a DECLINE-witness (oracle = correct
fold 5/3, auto-flips to pass when the freeze-once fold covers it). 

## FULL FOREIGN-PERFORM-IN-ARM BOUNDARY MAP (as of tick 1899)
FOLD (pass-witnesses): pyfv1 (inline in VALUE), pyfb3-valonly (let-bound in VALUE),
  pyfb2-discarded (do-stmt value dropped), pyfb4-single (let-bound-k-in-next-state at
  SINGLE dispatch), pytf2-ans (tail answer), + as7 (v-effects single-dispatch lib test).
DECLINE (safe, decline-witnesses): pyfb5-inline-nextstate (inline in NEXT-STATE, as2 guard),
  pytf1 (tail bare-foreign next-state coverage-gap).
MISCOMPILE (open, v-effects freeze-once fix pending): pyfb1 (let-bound-k in BOTH holes),
  pyfb3 (let-bound-k in NEXT-STATE) — both multi-dispatch, let-hoisted, binder-in-next-state.
