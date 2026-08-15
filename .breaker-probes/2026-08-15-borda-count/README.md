# brd1 — Borda count over three candidates (2026-08-15, tick 1514)

3-tuple points state with let-free helper defs (pts-of, bump — both pure
selectors/updaters over the tuple by index): `ballot(fst,snd,trd)` awards
2/1/0 via two chained bump calls, the rebuilt state bound once through a
match binder (rps2 idiom — call compound, so the kgt0 if-scrutinee wall
doesn't apply); `lead` answers the argmax with ties to the lowest id via a
comparison lattice.

Seed steers ONE ballot's first choice (n%3: 1 vs 0): n=10 → 3-way TIE 3/3/3
(lead falls to id 0 by tie-break... rows 2,3,1,3,0) vs n=0 → runaway 5/1/3
(rows 2,4,0,3,0). A 3-arg op exercises multi-parameter dispatch alongside.

PASS ×3 wasm. **Pool — fills ldg1/cfr1/brd1 (third trio ready).**
