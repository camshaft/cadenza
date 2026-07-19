# inc-4 multi-task run-sim — VERIFIED substrate shapes (trunk e6fd73bfa, tick 112)

All of these PASS on trunk today (probed, cleaned up). The full 2-worker run-sim assembles from them.
Landed reaches: step-3 escaping-k, single-payload extract, multi-payload tuple pop (all on trunk).

## PASSING building blocks (each verified → its expected value)
1. **Single pqueue pop + tail-resume** (PROMOTED to corpus, 24/0):
   `(match q ((PQCons (tuple wake kb rest)) (match kb ((KBox k) (k unit)))))`, k = `(fn (_u) (resume unit wake))`. → 5e9.
2. **Handler state = pqueue**, sleep files k into a queue, resume closes over `s`: `(resume unit s)` → PASS.
3. **spawn arm** with a pure child thunk `(fn (_u) unit)` → PASS.
4. **spawn + child thunk that itself performs `sleep`** (2nd suspension point) → PASS.
5. **spawn child-sleeps + main also sleeps** (two continuations both filed), constant grade → PASS.
6. **Compound state `(St (tuple clock pqueue))`**: `now` reads `(clk s)`; `sleep` arm resumes with a NEW
   state whose clock is advanced to `wake`: `(resume unit (St.St (tuple wake ...)))`; `(now)` after → 5e9. PASS.

## The remaining DESIGN subtlety (not a compiler decline — a scheduler-structure question)
The resume-thunk is CREATED at file-time (inside the sleep arm) but must resume with the state the
scheduler computes at POP time (clock advanced to that entry's waketime, pqueue = the rest). A thunk that
closes over `s` captures FILE-time state. Two ways to thread the pop-time state:
  (a) the boxed continuation takes the new-state as data the scheduler supplies at apply — BUT a resume-thunk
      taking a lambda PARAM for its new-state hit CDZ0101 earlier (resume-in-lambda-with-param). AVOID.
  (b) TAIL-CPS: sleep files (wake, k) where k closes over the info it needs (its own wake), and tail-calls
      scheduler-step; scheduler-step pops-min, and the RESUME happens with a state built from the popped
      entry's waketime. The clock-advance rides in the resume's new-state arg = `(St (tuple wake rest))`,
      computed where the pop happens (shape 6 proves this folds for a single entry).
The open piece: for MULTIPLE entries, scheduler-step must, after resuming entry-1's k (which runs that task
to its next sleep — re-entering scheduler-step via the sleep arm), continue to entry-2. Because each resume
is TAIL and re-enters via the resumed task's own next perform, the loop threads WITHOUT a drain-after-resume
(the recursive `(do (k unit) (drain rest))` shape DECLINES and is the wrong model anyway). So scheduler-step
= pop-min → tail-resume; the resumed task's next sleep files ITS next event + tail-calls scheduler-step again.
Termination: when a task finishes (no more sleeps) its k returns; the LAST primary's return is the answer.

## NEXT-TICK BUILD PLAN (assemble from verified pieces)
- State `(St (tuple clock pqueue))` (drop the trace UInt64 for v1; grade the FINAL CLOCK 5e9 first, add the
  woken-order "B,A,main" witness as a follow-up case once the scheduler threads).
- sleep arm: `(sleep (wake) s <file (wake, k) into (pq s); tail-call scheduler-step with clock unchanged>)`
  where k = `(fn (_u) (resume unit <state with clock advanced to this entry's wake>))`.
- scheduler-step(st): pop-min from `(pq st)`; if empty → return (sim done); else set clock:=wake, tail-resume
  the popped k with the new state.
- FIRST corpus case: 2 workers (A@3s via spawn, B@1s via spawn) + main@5s, grade the FINAL observed clock 5e9
  (simplest correct witness). THEN a second case grading woken ORDER (B,A,main) via a trace accumulator.
- If assembling these passing pieces into the loop DECLINES at a new shape, minimal repro → v-effects.
- Gate all 3 backends; surgical baseline inserts; MR.
