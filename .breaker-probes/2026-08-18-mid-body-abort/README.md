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
