# Map-of-Maps handler state (2026-08-10)

Angle: 05-compound pins CHAMP-in-CHAMP as a plain value; NOTHING pins it as
HANDLER STATE (the counters-by-category shape threading through dispatches).

GREEN x3, python-modeled first:
- mm1: (Map Int64 (Map Int64 Int64)) state; each bump dispatch rebuilds
  outer+inner (persistence across resume), returns the OLD cell; two-level
  drain reads after three bumps — 5000162/112
  (n=0 pin slip caught by the model pre-gate: read(1,10) is n+1 not n... the
  first bump writes old+1; corrected 162 -> 112.)

Pin candidate: the nested-CHAMP state face is the missing composite between
the landed flat-Map states and 05's value-position Map-of-Maps.
