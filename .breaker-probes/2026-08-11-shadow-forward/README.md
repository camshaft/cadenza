# Shadow-forward: inner same-effect arm re-performs to the outer (2026-08-11)

Angle: the landed forward pins are two-effect (Inner arm -> Outer effect) or
the no-home reject. An inner arm of a SAME-effect shadow re-performing the
effect it discharges (routing OUTWARD past itself) was unpinned in the
resume-value and next-state positions.

GREEN x3, python-modeled first:
- sf1: the re-perform sits in the RESUME VALUE — (+ (St.get) t); outer strides
  +1, inner +10, both states advance independently — 114103/111100
- sf2: the re-perform sits in the NEXT-STATE — (+ t (St.get)); the forward
  lives in the state-thread position (consumed by the later dispatch, so it
  evaluates per the strict family) — 103100/100100

Pin candidates: 235 pool.
