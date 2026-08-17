# prm1 — prism bench refracting a beam (2026-08-17, tick 1706)

Attack: two SENTINEL-VALUED compounds guarding their own rebuilds — aim's
mod-7 exit where ZERO means absorbed (the compound tested against its own
zero, absorbed branch keeps the OLD beam — the compound's value discarded
exactly when it hits the sentinel), and split's floor-div where zero means
dark (re-lit to a constant 5). Both ops' live branches thread the tested
compound; both sentinel branches discard it. The re-light is ftn1's
reset-to-constant on the OTHER side of the test.

Differential: beam 5 vs 3: n=10's first aim ABSORBS (5+3=8... (5+3)%7=1 —
no: beam 5, aim 3 → 8%7=1, live 13; model says [13,701,24,12] — the SPLIT
goes dark on n=10 (701) vs n=0's absorb at aim #2 (901). Each bench hits a
different sentinel; reads 120 vs 121.

Hand model: n=10 → 137010240120120; n=0 → 630319010120121 (mixed base;
first tuning converged after row 1 — re-aimed so the sentinels split).

Pass ×3 wasm + rust + rust-async on trunk 0657b816d.
