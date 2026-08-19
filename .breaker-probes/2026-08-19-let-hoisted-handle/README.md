# let-hoisted-handle — dispatching nested handle LET-HOISTED before resume

## Motivation
pyth1 established: a dispatching nested handle in the post-resume TOLL position
miscompiles (fixed by 7bc8916f9, narrowed to dispatching-only by eb654f126). This
bank tests whether **let-hoisting** the dispatching handle (evaluating it eagerly into
a `let` binding BEFORE resume, then using the binding post-resume) changes the outcome
— i.e. does the guard track the value's USE position (post-resume additive) or its
syntactic EVALUATION position (a let-RHS, not directly in the toll)?

## pylh1 — dispatching handle let-hoisted before resume
```
(tick () s
  (let ((k (handle E 40 ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2))))  ; k = dispatching handle = 42
    (+ (resume (+ s 1) (* 10 s)) k)))                                        ; k used post-resume
```
Model: 196/95 (same as pyth1 — the value still lands post-resume).

## pylh2 — NON-dispatching discriminator (inner body constant = 7)
Same but inner handle body is `(: 7 Int64)`, performs nothing. Model: 126/25.

## Observations (on trunk 7bc8916f9, the BROAD toll fix)
- pylh1 (dispatching, let-hoisted): DECLINES uniformly wasm+rust+rust-async.
- pylh2 (non-dispatching, let-hoisted): DECLINES uniformly (broad guard catches any
  toll handle).
Both decline under the broad guard — as expected, that guard is position-syntactic.

## OPEN RISK for the narrowed guard (eb654f126, dispatching-only, not yet landed)
The narrowed guard restores non-dispatching to a pass (pyth2 -> 126/25). The question
this bank raises: does the narrowed dispatching-check reach the LET-HOISTED dispatching
handle (pylh1), or only a handle SYNTACTICALLY in the toll?
- If narrowed guard tracks value-FLOW (a dispatching handle's value reaching the
  post-resume position, however bound) -> pylh1 correctly DECLINES, pylh2 PASSES.
- If narrowed guard is syntactic (handle-directly-in-toll) -> pylh1 may SLIP THROUGH
  and re-expose the 1414-class silent MISCOMPILE via the let-hoist.
Flagged to v-effects to verify their narrowed guard covers the let-hoisted value-flow.

## RESOLVED — narrowed guard landed (eacbabbf7), NO slip-through
The narrowing (dispatching-only toll decline) landed as **eacbabbf7** ("gate the pyth1
toll-handle decline to DISPATCHING-only — pyth2 folds"). Verified on a fresh post-land
build (tick 1872):
- pylh1 (dispatching, let-hoisted): DECLINES cleanly — **no 1414-class slip-through**.
  The guard recurses the whole arm-body subtree (per v-effects), so a let-hoisted
  dispatching handle is caught structurally.
- pylh2 (non-dispatching, let-hoisted): now PASSES 126/25.
So the gap-check I raised is CLOSED: the narrowed guard tracks the dispatching-handle
structurally regardless of let-hoisting. pylh1 stays a decline/todo-witness (oracle
196/95, flips to pass on the durable correct-fold); **pylh2 is now a PASS-witness**
(promotable). No new finding.
