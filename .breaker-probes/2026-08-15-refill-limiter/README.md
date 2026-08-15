# rrl1 — refill-on-read rate limiter (2026-08-15, tick 1568)

SCALAR tokens, single op: every `take` first refills by the seed rate
((n%3)+1) clamped at 10, then serves or answers the negated shortfall with
the refilled tokens KEPT. The clamp branch handles overflow (tokens+rate>10)
separately, so the arm is a 2x2 grid (clamped × sufficient). A final take(0)
reads the surviving balance (0-cost serve).

The refill compound (+ tokens rate) recomputed per branch — cheap, scalar,
6 dispatches: envelope-safe. Faster drip keeps refusals shallow (-3,-3)
vs the slow drip starving deeper (-5,-7) on the SAME requests.

vs odf1 (refill-as-op) and lky1 (leak-on-drain): this pins refill fused
INTO the serving op — the third bucket-discipline face.

PASS ×3. **Pool (12th trio seed).**
