# Run-length tracking (2026-08-12)

Angle: a 3-TUPLE state (last, run, best) where the transition has a
conditional dependency chain — nrun depends on (v == last), best on nrun —
computed in a let INSIDE the match arm before the resume. The 3-field
conditional-chain transition was uncovered (2-field pair states are pinned;
ac1's (prev,hits) is the 2-field cousin).

GREEN x3:
- rle1: seed 5 extends one run to length 5; seed 7 breaks it (run 3 of 7s) —
  5/3

Staged: 14c pool at 7 (pbr1/pbr2, sqm1, cz1, gcd1, fib1, rle1).
