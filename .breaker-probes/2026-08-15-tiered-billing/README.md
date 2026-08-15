# stp1 — tiered flat-fee billing (2026-08-15, tick 1556)

(usage, bills) state: `use` accrues answering the running usage; `bill`
answers the current tier's FLAT fee (5/<10, 12/<25, 20/≥25) resetting usage;
`total` reads collected fees. Seed pre-loads usage (10 vs 0): the FIRST
billing cycle lands in a different tier (18→12 vs 8→5), the later cycles
CONVERGE (both 20→12, 4→5), and the totals carry the difference (29 vs 22)
— a first-row divergence with converged tails, the mirror of dlt1's
converging stream.

3-branch bill arm, 2-tuple — envelope-safe. PASS ×3. **Pool — fills
tdr1/dth1/stp1 (11th trio ready).**
