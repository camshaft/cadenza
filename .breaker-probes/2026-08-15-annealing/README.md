# tmp1 — simulated-annealing acceptance schedule (2026-08-15, tick 1516)

(temperature, accept-count) state: `cool` decays temp by 9/10 truncating;
`accept` takes any improvement (d ≤ 0) and any worsening still under the
heat (d < temp), counting accepts; `tally` reads the count. Hot seed (80)
accepts every worsening move the cold seed (40) rejects — rows
1,72,1,64,1,57,1,1,5 vs 0,36,0,32,1,28,0,0,1 — while both take the improving
d=-3. The truncating 9/10 decay chain (72,64,57 / 36,32,28) rides alongside.

9 dispatches; branching arm (accept, 3-branch cheap) gets only 5 — inside
the envelope (trn broke at 7 with 4-tuple; this is 2-tuple).

PASS ×3 wasm. **Pool (4th trio seed).**
