# 2026-08-12 string rope-window (tick 1327, base cfedca65f)

- `srw1.sexp` — growing String state, computed interior slice window per dispatch:
  grow appends two chars via String.concat (rope), arm slices (lo, byte-len-1) with lo
  drawn from the perform args (computed (+ n 1)/(+ n 2)) and answers the window's
  byte-len via Option-match. String twin of the #21 shape (concat-rope producer +
  computed offset + Option-match in the arm) AND fresh String.slice-in-arm coverage
  (prior slices live in 13-strings bodies). Seeds flip window sizes: n=0 → 230, n=1 → 120.
  PASS ×3.
