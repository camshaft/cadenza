# 2026-08-13 lowest-terms fraction state (tick 1412)

- `fra1.sexp` — (num, den) kept in LOWEST TERMS: each addf cross-multiplies then
  renormalizes via an in-arm recursive Euclid gcd; the SEED itself normalizes
  through a let-with-gcd expression (n=2 seeds as 1/2). n=1 walks 1/4→1/2→1/1
  (exact collapse to unit mid-run: gcd 8/8)→7/6; n=2 walks 1/2→3/4→5/4→17/12.
  Composes: gcd-in-arm (gc1 pins Euclid as the whole arm; here it's a SUBROUTINE
  of a bigger transition), seed-position computation, and the tuple rebuild.
  PASS ×3 (10201010706/30405041712).
