# Pure helper call as the toll (2026-08-18)

- `pyj1.sexp` — the toll IS a def call: (+ (resume s (+ s 1)) (sq (+ s
  2))). The unwind routes through a real function call per frame (46 =
  fold 21 + sq(3)=9 + sq(2)=4... model: 21 + 9 + 16 = 46 for s0=1;
  23 = 10 + 9 + 4 for s0=0). Completes the call-rung symmetry: pyh1 put
  a recursive call around the RESUME VALUE (consumer side); pyj1 puts a
  call in the TOLL (producer side). Non-recursive here — a recursive
  toll helper is the natural follow-up if this area heats up. PASS x3
  at 2d2a3116f.
