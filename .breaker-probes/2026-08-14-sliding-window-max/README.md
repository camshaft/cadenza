# swm1 — sliding-window maximum, width 3 (2026-08-14, tick 1471)

Single-op handler over a List window: push appends, evicts the oldest via a
recursive drop-front rebuild once past width 3, and answers the max scanned
from the LIVE window by a second recursive def. Seeds disagree on which pushes
set new maxima (n=10: the first push 12 dominates until 8-then-16;
n=0: maxima climb monotonically 2,5,5,8,8,8).

Arm shape: nested dual-use lets (grown feeds trimmed feeds both slots) with
TWO recursive defs (dropf, maxs) inside one branch — the heaviest single-op
arm in the family, and it PASSES (single-op immunity holds even here).

Gated on the schema-hash phase-2 wire-flip base (584970864) with a fresh
build. PASS ×3 wasm. **Pool (batch-274).**
