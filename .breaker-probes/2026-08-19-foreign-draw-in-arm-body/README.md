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
