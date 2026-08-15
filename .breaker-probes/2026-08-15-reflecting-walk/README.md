# wlk — reflecting walk: 4-branch × 2 reflect walls declines; 1 wall passes (2026-08-15, tick 1539)

Reflecting random walk between walls 0/6, direction from bit 1 of a hidden
LCG. (pos, seed) 2-tuple.

| probe | shape | verdict |
|-------|-------|---------|
| wlk1 (3-tuple w/ max) | +peak high-water | DECLINE ×3 |
| wlk1 (2-tuple, binder) | s2 match binder + 4-branch reflect lattice | DECLINE ×3 |
| wlkB | binder dropped, LCG inlined ×4 sites | DECLINE |
| wlkC | ONE wall (floor only), 3-branch | **PASS ×3** |

The 2-tuple decline needs the FOUR-branch reflect lattice (both walls);
dropping to one wall (3 branches) compiles. Combined with the bnf/phs family:
the frontier is not just tuple width — branch count × cross-field guard
structure interact. Not separately routed (same family, banked datapoint).

wlkC (single-wall face) is corpus-eligible: same LCG stream, parallel tracks
two apart, floor bounce only on the low seed. PASS ×3. **Pool.**
