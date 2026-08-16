# Deep concat chains (2026-08-11) — 05-compound target (hold-safe)

Angle: a left-leaning 10-deep concat chain built by RECURSION (one element per
concat) — the pathological rope-tree shape; random-access reads at head/middle/
tail must stay exact through whatever rebalancing/representation applies.

GREEN x3:
- dc1: grow(10, [n]) via (concat acc [k]); len + reads at 0/5/10 —
  1170601/1100601

Pin candidate: joins lc1/as1 in the 05 batch pool.
