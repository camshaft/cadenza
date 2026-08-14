# 2026-08-14 tortoise-and-hare (tick 1453)

- `tth1.sexp` — Floyd cycle detection with the successor function BEHIND the
  effect: the arm computes (2i+n)%6 and counts calls; the body's recursive
  driver advances slow one succ and fast two (a NESTED perform — succ of succ —
  in the fast leg), meeting inside the cycle. The call tally (3 per iteration)
  rides as the final digit via a second op. Budget k=8. Seeds change the cycle
  structure (n=1 meets at 3 after 2 steps w/ 6 calls; n=2 at 0 with 6 calls).
  Nested-perform-in-argument + two-speed driver + meet-detection. PASS ×3
  (2036/2006).
