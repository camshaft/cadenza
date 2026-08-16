# Per-branch resume strides (2026-08-12)

Angle: the arm resuming under a CONDITIONAL where each branch carries a
DIFFERENT value AND stride — the landed per-branch pins vary the value only;
divergent strides make the branch choice change all downstream routing.

GREEN x3:
- pbr1: if-branch resumes with (v*10, +1) vs (v+100, +50) — the else branch's
  50-jump flips the NEXT dispatch's route — 30030/30103/30103
- pbr2: THREE-way match-branch resumes by residue class, each with distinct
  answer shape and stride; three dispatches walk the classes —
  29999900/-370001/800502

Staged for the next 14c batch.
