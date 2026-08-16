# rds1 — recursive-driver shared-let (2026-08-16, tick 1616)

Third and final probe into the tpwJ deferred classes (xhs1 cross-handler:
MISCOMPILES, filed; gws1 growing-state: passes correctly).

## Shape
A recursive driver (`drive`) keeps pulling until the arm answers a -1 stop
sentinel; the pull arm let-binds `v2 = cur + 6 + bias`, uses it in the stop
test, the answer, and the threaded next-state. Seed bias makes one run stop a
hop earlier (accumulated trails differ in length: 4 hops vs 5).

## Result: CLEAN DECLINE, binder-independent
- rds1 (with binder): declines — "this handler is not yet reducible by the
  tail-resumptive fold (cross-function or non-tail resume arrives in a later
  increment)". Uniform gate todo ×3.
- rds1-nolet-control (binder inlined ×3): declines IDENTICALLY.

So for the recursive-driver class the decline comes from the DRIVER's
cross-function resume shape itself (drive's match-over-perform), not the
shared-let — v-effects' clean-decline claim HOLDS for this class (unlike
cross-handler). No wrong answers possible; safe floor intact.

Both banked as todo-witnesses for the "later increment" cross-function fold.
Deferred-class audit COMPLETE: 1 miscompile (xhs1, filed + being fixed),
1 correct (gws1, staged pass-pin), 1 clean decline (rds1, todo-witness).
