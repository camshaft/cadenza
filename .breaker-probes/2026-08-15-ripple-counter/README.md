# rpc1 — ripple-carry counter (2026-08-15, tick 1509)

SCALAR mask state, branch-free arms: `inc` answers popcount(old XOR new) —
the number of bits flipped by that increment, the classic amortized-analysis
witness (carry chains: 10→11 flips 1, 11→12 flips 3, 15→16 would flip 5);
`pop` reads the live bit count via the recursive `bits` def (shared with the
bms1 family style, let-free).

Seeds 10 vs 0 fire the carry chains at different steps: flip rows
1,3,-,1,2,-,1 vs 1,2,-,1,3,-,1 and the pop rows differ (2,3,4 vs 1,3,2).

F24-safe: scalar state, ZERO branches in arms, recursion lives in the callee.
8 dispatches. PASS ×3. **Pool.**
