# recursive-driver-pyfb3 — v-effects' freeze-once recursion-guard boundary case
## Ask (v-effects): a pyfb3-shaped arm where the foreign draw is at ONE static site but the
## OUTER driver recurses so the handler dispatches the inner op N times at runtime — to verify
## the static-count gate's recursion guard. Correct oracle must be collapse-fold OR clean-decline,
## NEVER the triangular miscompile.

## RESULT: the exact shape is REJECTED AT SCOPE-CHECK (both natural routes) — the hole appears
## unreachable in this surface.

Three attempts, all scope-rejected (NOT the fold, NOT a miscompile — front-end rejects):
1. `toplevel-def-2handler.sexp`: top-level `(def (drive d) ... (A.tick) ...)`, A inner of two
   handlers, A's arm does `(let ((k (B.beat))) (resume (+ s 1) (+ s k)))`.
   → "this effect operation is performed with no enclosing handler here; its home is determined
      by the handler or delegation enclosing its callers" (the shared top-level def performing the
      inner effect has no static home).
2. `toplevel-def-1handler-selfop.sexp`: single handler A with two ops (tick/beat), drive performs
   A.tick, arm's next-state reads a let-bound A.beat. → SAME "no enclosing handler" reject.
3. `self-recursive-arm.sexp`: arm re-performs its own op `(A.run (- d 1))` to self-drive N
   dispatches. → CDZ0401 "reached with neither an enclosing handler nor a host delegation" (an arm
   runs outside its own handler; re-performing its effect there has no home).

CONTROL (`CONTROL-recursive-drive-pure-arm-COMPILES.sexp`): the SAME top-level recursive `drive`
performing A.tick, but A's arm is pure `(resume (+ s 1) (+ s 1))` (no let-draw) — COMPILES + runs
25 at n=5. So the recursion itself is fine; it is specifically the arm PERFORMING (the let-draw /
self-op) under a driven-from-a-shared-def perform that trips the home-inference reject.

## CONCLUSION for v-effects
The runtime-N-dispatch-with-static-count-1 recursive-driver pyfb3 is UNREACHABLE via the two
natural routes (top-level recursive def performing the effect; self-recursive arm) — both fail
front-end scope/home checks BEFORE the fold runs. So the static-count gate's recursion-guard hole
may be moot in practice. CAVEAT: this is a scope-check ACCIDENT, not a fold guarantee — if a future
front-end change admits cross-function performs under nested handlers (or delegation makes the home
inferrable), the hole reopens. Recommend v-effects keep the conservative "decline if can't prove
single-dispatch" rather than rely on the reject. No miscompile reachable today.
