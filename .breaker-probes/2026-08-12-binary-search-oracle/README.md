# 2026-08-12 binary-search oracle (tick 1349, base post-243 trunk)

- `bis1.sexp` — BINARY SEARCH as the effect protocol: the handler holds the hidden
  target and its arm only answers a -1/0/1 verdict; the body's recursive driver
  narrows (lo,hi) from the verdicts. DATA-DEPENDENT dispatch count per seed
  (n=37: probes 50,25,37→found in 3; n=50: found in 1), budget-guarded (k=5),
  trace digits + final interval width pin the descent. The oracle inversion —
  handler-as-adversary, body-as-algorithm — complements the algorithm-trace pins
  (cz1/gcd1/fib1) where the ALGORITHM lives in the arm. PASS ×3 (13224 / 300).
