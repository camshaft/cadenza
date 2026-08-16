# Atomic slot swap (2026-08-12)

Angle: a swap dispatch exchanging BOTH tuple fields in one transition —
(a,b) -> (b,a) with the resume value reading both — plus reads before/after.
A non-atomic exchange (write a then read it as the new b) would double one
field; the n=0 seed makes the corruption face n-visible.

GREEN x3:
- swp1: 710007/10000

Staged: 14c pool at 9 (…, mia1, swp1). Batch-236 more than ready.
