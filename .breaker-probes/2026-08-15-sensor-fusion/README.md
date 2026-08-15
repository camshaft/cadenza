# sfu1 — two-sensor weighted fusion (2026-08-15, tick 1503)

(estimate, weight-sum) state: read1/read2 fold a sample into the running
weighted mean — est = (est*wsum + v*w)/(wsum+w) truncating — with read1 at
fixed trust 3 and read2 at seed-shaped trust (n%4)+1; wtot reads the
accumulated weight. The fused-mean recurrence exercises multiply-then-divide
on the state thread with the SAME compound expression in both resume slots
(dual-use-by-recompute, branch-free arms).

Trust 3 vs 1: the second sensor pulls the estimate hard (16 vs 14 after its
first read) or barely (13 vs 10 at the end); wtot differs (12 vs 8).

F24-safe: 5 dispatches, ZERO branches in read arms, 2-tuple. PASS ×3. **Pool
(with dbt1; +1 fills the trio).**
