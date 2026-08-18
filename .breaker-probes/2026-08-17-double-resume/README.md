# Sequential double resume (2026-08-17)

- `dbr1.sexp` — TWO resumes in sequence in one arm: (do (resume s (+ s 1))
  (resume (+ s 10) (+ s 2))). Sweep: zero corpus cases sequence a second
  resume after the first returns (the 545 multi-resume arms are all
  branch-exclusive — one resume per path). Verified UNIFORM x3: the
  continuation is MULTI-SHOT and the second replay's value wins (main(10):
  first replay returns 1, discarded; second replays the tail with 11 ->
  answer 11; main(0) -> 10). Neither an error nor a no-op — a real
  semantics pin for the one-shot-vs-multi-shot question, adjacent to the
  F24 exponential-duplication family (F24 duplicates via FOLD lowering of
  branch-exclusive resumes; dbr1 pins the SEQUENTIAL case at n=2 replays,
  single-perform body so the replay cost is linear). PASS x3 at 07ebd60a6.
- `dbr2.sexp` — double resume over a TWO-perform body: 2 dispatches x 2
  replays = 4 leaf executions; second-replay-wins composes MULTIPLICATIVELY
  (the surviving answer threads replay-2 at BOTH depths: main(10)=141 =
  a-replay2(11) + 10*b-replay2(13)). Replay growth is 2^depth — dbr2 pins
  depth 2; a depth explosion would show as a hang here first. PASS x3 at
  07ebd60a6. Hand-model uses a nested-function simulation of the replay
  tree (see ledger tick 1739).
- `dbr3.sexp` — BOTH replay values CONSUMED: (+ (resume s ...) (resume
  (+ s 10) ...)). Neither replay discarded — the arm's answer is the two
  body values summed (main(10): s0=1 -> 1 + 11 = 12). Completes the
  sequential family: discard-second-wins (dbr1), composition (dbr2),
  both-contribute (dbr3). My first oracle write was stale (11 vs actual
  12) — the --case actual matched the python model, the .sexp had the
  typo; fixed pre-bank, model re-run rule held. PASS x3 at 07ebd60a6.
