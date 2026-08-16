# Parity-alternating two-effect draws (2026-08-12)

Angle: ONE recursive driver alternates WHICH effect it draws from per hop
(loop-counter parity picks A or B) — the same-op-name two-effect interleave
inside a single recursion. Same-name ops (A.get/B.get) also pin name
resolution across effects in one scope.

GREEN x3:
- pal1: 4 hops alternating B,A,B,A (k=4..1) with positional weights —
  6072/13142

Staged: 14c pool at 9. rm1 pruned this tick (v-effects has the shape QUEUED).
