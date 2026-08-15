# lfs1 — 8-bit Fibonacci LFSR (2026-08-15, tick 1519)

SCALAR register: `step` shifts right injecting XOR(bit0, bit2) as the new
high bit, answering the register — the whole shift-XOR-inject compound
recomputed in both slots (scalar dual-use-by-recompute, branch-free);
`peek` masks the low nibble. Seeds 5 vs 3 fall into DIFFERENT orbits:
5 → 2,1,·,128,64 (decays then rings the injected ladder) vs
3 → 129,192,·,96,48 (rings the high bits immediately) — every row differs,
including both peeks (1/0 vs 0/0 at different points).

Branch-free scalar at 6 dispatches — envelope-safe. PASS ×3. **Pool —
fills tmp1/chg1/lfs1 (fourth trio ready).**
