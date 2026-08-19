# A whole handle expression as the toll (2026-08-19)

- `pyre1.sexp` — the post-resume toll IS a fresh handle over the SAME
  effect, installed during the unwind: (+ (resume ...) (handle E 7 ...
  (E.tick))). The toll-region's draw answers 107 from its own seed;
  the unwinding frame's fresh region neither reads the outer state nor
  leaks its own (117 / 107). The unwind-side dual of hoh1 (handle in
  INIT: a region before the frame exists) — here a region lives inside
  a frame's DYING moments. Completes handle-expression positions:
  INIT (hoh1), body summand (pyn1), and now the TOLL itself. PASS x3
  at 3d3ef1d49.
- `pyre2.sexp` — the toll region is SEEDED from the dying frame's
  capture: (handle E (+ s 20) ...) inside the toll. The capture flows
  INTO the region (52 = 10*s0 + 2*(s0+20)); nothing flows back. A
  region seeded from post-resume state would shift by the advance.
  Completes the handle-as-toll pair: pyre1 isolated (own seed), pyre2
  capture-fed. PASS x3 at 0aa1396b4.
