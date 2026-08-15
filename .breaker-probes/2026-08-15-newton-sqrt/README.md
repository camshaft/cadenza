# nsq1 — Newton integer square root (2026-08-15, tick 1505)

(x, t) state: `improve` does one Babylonian step x ← (x + t/x)/2 from the
high start x=t, answering the shrinking iterate (nested truncating divisions);
`done` checks the bracketing invariant x² ≤ t < (x+1)² via a 3-branch
nested-if. Seeds t=130 vs t=60 CONVERGE AT DIFFERENT SPEEDS: the second done
probe answers 0 (130: x=12, 12²=144 > 130 — overshot below) vs 1 (60: x=7,
49 ≤ 60 < 64 — done); the third answers 1 on both (11²=121 ≤ 130 < 144).

F24-safe: the branching arm (done) receives only 3 of 8 dispatches (≤4 rule);
improve is branch-free.

PASS ×3 wasm. **Pool (next trio seed after dbt1/sfu1/trb1 ships).**

## Rust backstop VERIFIED (tick 1533)
884d37ba3 (rust per-function size backstop): nsq1 flips artifact-did-not-build
→ clean decline on BOTH rust targets; wasm still passes. Chain-multiply
witness now decline-today, rides (b) for the compute pass-pin.
