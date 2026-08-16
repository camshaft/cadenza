# wrd1 — digit-ladder distance scorer (2026-08-15, tick 1574)

(target, best) state with a let-free recursive digit-walk callee (ddist:
compares k digit pairs via %10 / /10 recursion): `feed` answers the
per-digit Hamming distance to the seed-picked target through a match binder
over the CALL compound (fence-safe), tracking the minimum; `closest` reads.

Targets 345 vs 375 rank the SAME guesses differently — each run holds a
different exact-hit row (row 1 vs row 3 = 0), the 325 row ties at 1 on both,
and 340 splits 1-vs-2. The packed totals differ in LENGTH (leading zero on
n=10's total: 1010100 vs 101000200) — a leading-zero-row packing face.

PASS ×3. **Pool (11th trio seed... recount: 10 trios + wrd1).**
