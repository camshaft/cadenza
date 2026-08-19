# Sequential same-effect regions with different toll rates (2026-08-19)

- `pysq1.sexp` — two TOP-LEVEL handles over one effect, one after the
  other: independent seeds (n%3 vs 5), arms, and toll rates (x100 vs
  x200). The tolls never cross (1050110 / 1050000); a stale frame or a
  shared arm table from region 1 misprices region 2. The top-level
  SEQUENTIAL complement to pysh7 (siblings inside one outer) — no outer
  handler exists here at all, so the second region installs from a
  clean slate. PASS x3 at 3d3ef1d49.
- `pysq2.sexp` — a TOLLED region then an UNTOLLED one over the same
  effect: region 1's two frames unwind their x100 tolls fully before
  region 2 installs a plain tail-resumptive arm; region 2's draws fold
  clean (110330 / 110110). Toll infrastructure leaking across the
  boundary inflates the ten-thousands. Notable vs the MIXING boundary
  (pyx5 README): tolled/untolled arms in ONE effect-instance decline,
  but tolled and untolled REGIONS in sequence are fine — the mixing
  constraint is per-handle, not per-effect-type. PASS x3 at 3d3ef1d49.
