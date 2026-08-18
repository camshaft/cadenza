# Post-resume toll keyed to the op argument (2026-08-18)

- `pyv1.sexp` — the toll reads the OP ARGUMENT, not the state: (tick (v) s
  (+ (resume (+ v s) (+ s v)) (* 100 v))). The two dispatches pass
  different arguments (4 then 7), so each unwinding toll must recall the
  argument ITS dispatch received across the suspend (1225 = fold 125 +
  700 + 400). Completes the capture-set coverage for post-resume
  expressions: state (pyr1), tuple-state binders (pyr8), and now the op
  ARGUMENT. A toll reading the other frame's argument or the state
  shifts the hundreds. PASS x3 at c8c5cb63e. Note: cdz-smith commit
  c8c5cb63e ("multi-shot double-resume arm reach") landed this tick —
  the smith is now generating in dbr territory two days after my pins.
