# Sibling shadow regions share one outer thread (2026-08-18)

- `pysh7.sexp` — two SIDE-BY-SIDE handles over the same effect, each
  self-performing to the shared outer arm: the outer state advances
  through the first sibling INTO the second (consecutive rungs 10*s0+1
  then 10*(s0+1)+1: 821061 / 811051). A fork-per-region or reset-
  between-siblings collapses the rung gap. The SIBLING complement to
  pysh2 (sequential uninstall) and pysh6 (nested chain) — the shadow
  topology map now covers nested, sequential, and sibling layouts.
  Design note: draft 1 (pure sibling arms, no self-performs) produced
  seed-INDEPENDENT output — the shadows fully masked the seeded outer;
  the self-performs are what expose the shared thread. PASS x3 at
  b7972ffd6.
