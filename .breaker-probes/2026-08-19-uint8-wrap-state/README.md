# uint8-wrap-state — UInt8-typed handler state × multi-dispatch DECLINES (coverage gap)
## pyu8w1 — UInt8 state threaded by UInt8.wrapping-add across 3 dispatches. Model 251000005/250255004. DECLINES (todo) x3.
"this handler is not yet reducible by the tail-resumptive fold". ISOLATED:
- M1 (UInt8 state, multi-dispatch, even a pure UInt8.wrap next-state): DECLINES.
- M2 (UInt8 state, SINGLE dispatch): compiles.
- M3 (Int64 state, same multi-dispatch shape): compiles + RUNS correct (250255).
- M4 (Int64 state, UInt8.wrap in the COMPUTATION but Int64 thread): compiles.
=> The gap is SPECIFICALLY a NARROW-INT (UInt8) HANDLER STATE threaded across >=2 dispatches;
Int64 state folds, single-dispatch UInt8 folds, UInt8 arithmetic in the body folds. Likely
related to the width-alias class (#16-23). SAFE over-decline (reject, not miscompile).
Filed to v-effects. Held as a decline-witness (oracle 251000005/250255004, auto-flips on fold-extension).
