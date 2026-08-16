# lok1 — canal lock filling toward a seed pool (2026-08-16, tick 1594)

(lock-level, trips) state: `enter` answers the full gap to the seed pool
(n+6: 16 vs 6); `equalize` raises the lock 2 clamped at the pool; `exit`
answers 101+trips on a level MATCH or the SIGNED gap (lock−pool, negative
while below). The low pool completes passage on the third equalize (exit
101) while the high pool is still 12 short at the same row (−12 then −10) —
the same op sequence ends in success on one run and repeated refusal on the
other, with the equalize ladder rows shared (2, 4, 6 both).

PASS ×3. **Pool — fills tid1/jmp1/lok1 (12th trio ready).**
