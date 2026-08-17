# Handle whose INIT is a whole handle expression (2026-08-17)

- `hoh1.sexp` — sweep: `(handle X (handle` count 0 in corpus (40 op-calls
  and 6 ifs appear inside handle INITs, 1 let, 0 matches, 0 nested handles).
  The outer Fibonacci pair-walker's starting tuple is computed by an INNER
  two-dispatch counter handle: (handle B (% n 3) (...) (tuple (B.step)
  (B.step))) sits in the outer handle's INIT slot. The inner handler answers
  seed+1 while doubling its own state (so the two B.step draws differ), and
  must be fully torn down before the outer F installs — a frame-lifecycle
  ordering probe at the INIT boundary rather than the usual body/arm sites.
  4/4 rows diverge across n%3 seeds. PASS x3 at c0be8a856.
