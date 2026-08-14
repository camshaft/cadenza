# 2026-08-14 deterministic reservoir sampling (tick 1451)

- `rsv1.sexp` — reservoir-sampling structure made deterministic: an LCG
  (s*13+7 mod 101) threads BESIDE the reservoir in a 3-tuple (seed, kept,
  count); keep-or-replace decided by seed-mod-count == 0. Seeds route which
  offers displace (n=3: 10→20 stays; n=7: 10 holds then 30 displaces).
  Deliberately 3 DISPATCHES — the arm has 3 chained lets (c2/s2/k2, the k2
  dual-feeds resume+state) which per the tick-1448 cliff would DECLINE at 4+;
  at 3 it compiles and pins the LCG+decision composition. First LCG-in-arm.
  PASS ×3 (102020/101030).
