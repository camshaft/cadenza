# 2026-08-13 frequency decay (tick 1435)

- `dcy1.sexp` — every observe HALVES all counts (whole-map rebuild dropping rows
  that zero: (/ v 2) > 0 filter inside the rebuild walk) THEN bumps the observed
  key +4 on the decayed map. Composes fan1's whole-map transform with a
  conditional row-drop and a follow-up single-key update in ONE arm. Seeds:
  n=1 alternates keys (map holds 2, drops back to... rows 104/106/204/205);
  n=2 stacks one key (repeated halve+bump converges: 4→6→7→7 the fixed point
  of x/2+4). The convergence row (7→7) pins the integer fixed point. PASS ×3.
