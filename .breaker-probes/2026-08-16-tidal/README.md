# tid1 — tidal predictor on a triangle wave (2026-08-16, tick 1592)

SCALAR clock state with a let-free triangle-wave callee (levl: phase mod
2·amp, reflected above amp): `read` advances one hour answering the level;
`moor draft` answers 1 when the level covers the draft, else the negated
shortfall (levl called twice in the moor arm — guard + answer — both
single-branch, in-envelope).

Amplitudes 3 vs 2: the smaller peaks EARLIER, so the same moor probes catch
one tide still rising (level 2 → shortfall −1) and the other already ebbing
(level 0 → shortfall −3); rows share the rise prefix (1, 2) then split.

PASS ×3. **Pool (12th trio seed... recount: 11 trios + tid1).**
