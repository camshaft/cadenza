# Set algebra across dispatch (2026-08-11)

Angle: Set.union/intersection/difference never appear in 14-effects — the
arm-side set-algebra face was unpinned.

GREEN x3, python-modeled first:
- sa1: the probe arm builds a set from its ARGUMENT and answers
  100*|s∪arg| + 10*|s∩arg| + |s\arg| against the threaded state; a grow
  dispatch inserts between two probes so the SAME probe answers differently
  (311 then 321) — 3110521/3110521 (n-independent by construction)
  (model caught my first pin: probe after grow has intersection 2 not 1.)

Vocab: Set.empty does NOT exist (CDZ0201) — seed with (Set.of (list ...)).
Note Map.empty DOES exist; the two modules differ.

Pin candidate for 230-pool.
