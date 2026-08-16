# Min-tracking stack (2026-08-12)

Angle: a MIXED heap+scalar tuple state — (stack, min) where each push threads
the growing List AND the scalar min together in one transition; mid-run and
final min reads. The heap+scalar co-transition per dispatch is the face
(slmin11 pins tuple(scalar,string); the List+scalar pair with per-push heap
growth was uncovered).

GREEN x3:
- mns1: pushes 5,n,1 with min reads after 2 and 3 — 301/501

Staged: 14c pool at 12.
