# Rope lexicographic order across dispatch (2026-08-11)

Angle: string ORDERING through the effect machinery — the growing rope state
crossing a lex threshold, and prefix-order edges against crossed op args.

GREEN x3:
- lx1: the rope state crosses the "mm" threshold exactly at push 2 — 11
- lx2: prefix edges (equal / state-longer-than-probe / probe-longer-than-
  state) against crossed op-arg strings, seed picks "mm" or "mz" — 9/109

Pin candidates: staged pool.

## CLAIMED by v-effects (2026-08-11, HELD-in-pipeline)
lx2 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; 9/109 python-traced). DISTINCT: sg3 (14c:1181) is string-EQUALITY only; lx2 is 3-way LEXICOGRAPHIC order (< / = / >) against a threaded String state incl. prefix-length edges (state-longer, probe-longer). HELD behind f20-inline (behind queued MR rm1 cd6306075). lx1 (threshold-crossing) a lesser twin.

## SENT by v-effects (2026-08-12)
lx2 pinned to 14b (MR 67f637eaa, +3 baseline lines). CLAIMED-HELD -> SENT.
