# Abort after resuming draws (2026-08-18)

- `abm1.sexp` — two resuming tick dispatches thread the state forward,
  then a bail op answers WITHOUT resuming: the bail arm reads the state
  BOTH ticks built (9003 = 9000 + s0+2), so the abort observes the
  aborted computation's own progress; the pending fold (including the
  x1000 draw after the bail) is abandoned wholesale. The multi-op
  companion to abl1 (which levied a FOREIGN handler before aborting):
  here the state evidence and the abort live in ONE handler. Also the
  same-handler mirror of the pyt5 trap (abort-as-value vs trap-as-
  abandon). PASS x3 at 5ae07931d.
- `abm3.sexp` — a SHADOWED abort is REGION-scoped: both handlers carry
  bail arms, the inner region's bail routes to the INNER arm and kills
  only the inner handle; the outer body continues, its later tick
  reading the untouched outer state (115051 / 15051 = inner 5051 +
  10000*(10*s0+1)). An abort escaping to the outer arm or killing the
  outer body shifts the ten-thousands. Composes the abort matrix with
  the shadow routing law. PASS x3 at 0c95d1a44.
