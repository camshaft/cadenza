# lck1 — combination-lock sequence matcher (2026-08-15, tick 1521)

(cursor, opens) state with a let-free digit-at callee (literal-indexed
secret whose MIDDLE digit is seed-shaped): `press` advances the cursor on a
match, or falls back to 1 (answering -1) when the wrong press re-matches the
first digit — the classic KMP-style restart — else to 0; `opn` pays 101+opens
on a full match (resetting) or answers the negated cursor.

Secrets [3,3,7] vs [3,1,7]: the SAME eight presses open the lock in the
FIRST half on n=10 and the SECOND half on n=0 — the payoff row (101)
migrates across the packed total (102040101000000 vs 99000001020401), and
the KMP-restart row (-1) appears only on n=0.

3-branch press arm × 6 through it, cheap recomputes, 2-tuple — envelope-safe.
PASS ×3. **Pool (with knt1; +1 fills the 5th trio).**
