# cpd1 — compound-interest ladder (2026-08-16, tick 1583)

SCALAR principal, branch-free arms: `grow` applies the seed rate percent
truncating — the compound (+ p (/ (* p rate) 100)) recomputed in both slots
(2 consumers, safe per the tnk axis); `skim` withdraws. The two-point rate
difference COMPOUNDS: the gap between runs widens every grow row (4,8 →
12,16 across the skims) while the skims subtract identically — geometric
divergence under identical linear operations.

Growth rows also pin truncation: 5% of 190 = 9 (truncated from 9.5), giving
199 not 200 on the n=10 run.

PASS ×3. **Pool (with tnkB; +1 fills the 12th trio).**
