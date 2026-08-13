# 2026-08-13 compose-accumulating closure state (tick 1377)

- `cmp1.sexp` — the closure state is REPLACED each dispatch by a new closure that
  CAPTURES THE OLD ONE: f2 = (fn (x) (+ (* (f x) 2) d)), applied at 1 per step.
  After two dispatches the state is a two-deep composition chain whose innermost
  layer still captures the enclosing function's n. vs the landed closure-state
  pins (param-capture seed 14:3680; arm-binder replacement 14:3700): neither
  NESTS the previous closure inside the next — this pins the compose-accumulate
  idiom (env chain grows per dispatch; n=3: 8·1000+21; n=0: 2·1000+9). PASS ×3.
