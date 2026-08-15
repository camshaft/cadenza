# elv1 — elevator SCAN: F24 body-size, model-consistent (2026-08-15, tick 1515)

Elevator over floors 0-7: (floor, direction, request-bitmask) 3-tuple. call
ORs a request bit answering popcount; move advances one floor with reversal
at extremes and serves+clears a hit request (6-branch expanded lattice after
avoiding the kgt0 if-scrutinee wall). Seven dispatches, 4 through move.

INVALID WASM ×3 at both 5-move (8 dispatches) and 4-move (7 dispatches)
forms: 12,790,642-byte emit, body-size kind. Model-CONSISTENT: move is a
6-branch arm over a 3-tuple with per-branch compound recomputes ((+ floor d)
etc.) — dispatches(4) x branches(6) x (width(3) + recomputes) is far past
every passing point in the matrix. Not a new face — the cost model PREDICTED
this; recorded as the model's first successful prospective prediction rather
than a new finding. Witness banked; no new issue filed (v-rb already has the
acceptance shape).

Envelope note for probe design: 6-branch arms are effectively capped at ~2-3
dispatches until the fix lands.
