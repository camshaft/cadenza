# Negative zero as op argument (2026-08-11)

Angle: -0.0 crossing the dispatch boundary as an op argument (fx7 pins the
state thread; fa1 pins NaN/inf args; the signed-zero arg was uncovered).

GREEN x3:
- nz1: (* a 1.0) and (* a -1.0) at a=0.0 produce +0 and -0; the arm's
  canonical (= x 0.0) is TRUE for +0 but FALSE for -0 (canonical equality
  distinguishes signed zeros, consistent with fx7's doc) — the -0 arg takes
  branch 3, giving 13 not the IEEE 12. a=2.5 control: 33.

Semantics note (re-derived, consistent with fx6/fx7): Cadenza `=` is
canonical — NaN self-equal AND signed zeros DISTINCT. Both diverge from IEEE
in opposite directions; both now pinned in arg position (fa1 + nz1).

Staged for the next 14c batch.
