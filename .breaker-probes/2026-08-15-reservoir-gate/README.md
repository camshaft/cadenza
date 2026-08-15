# tdr1 — reservoir gate with ratcheting threshold (2026-08-15, tick 1554)

(level, threshold) state: `inflow` raises the level; `gate` releases HALF
truncating only above the threshold — which RATCHETS up by one after every
release (both state fields written by the release branch); a held gate
answers 0. The half-release compound (/ level 2) is recomputed in both slots.

Seed threshold (n%4)+4 (6 vs 4): the lower threshold releases on the FIRST
gate (2) where the higher holds (0); mid-stream both release but from
different levels (4 vs 3); the runs RE-CONVERGE on the shared row 6 (both
release 5) and the final held gate (0 both) — divergence, partial
re-convergence, and a shared tail in one stream.

2-branch arm, 2-tuple — envelope-safe. PASS ×3. **Pool.**
