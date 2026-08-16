# prs1 — twin parking meters (2026-08-16, tick 1587)

(m0, m1) state with let-free dec1/exp1 callees: `feed i c` adds c×seed-rate
to one meter (rate (n%3)+2: 3 vs 2); `tick` decrements BOTH clamped at zero
answering the expired count — the tick arm calls dec1 twice for the state
AND exp1∘dec1 twice for the answer (four callee calls per dispatch, all
let-free single-branch — in-envelope).

Rates 3 vs 2: meter 1 expires 2 ticks earlier on the slow rate, walking the
expired count 0,1,1,2,2 vs 0,0,1,1,1 — a two-meter race where the answer is
the COUNT of finished racers, and the double-clamp (0/0) holds the count at
2 (expired stays expired).

PASS ×3. **Pool (12th trio seed).**
