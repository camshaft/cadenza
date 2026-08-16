# tpx1 — chained shared-lets through a 3-way arm (2026-08-16, tick 1619)

Attack: stress the tpwJ collapse (8276ad1a6) past its landed pin. The arm
chains TWO binders (c2 from state+arg+bias, d2 derived from c2 AND the raw
state), routes on d2%3, and each of the three branches resumes a DIFFERENT
mix: (d2-answer, c2-state) / (c2-answer, d2-state) / (c2+d2, c2+1). A peek op
reads the threaded column. If the all-or-nothing collapse gate mis-fired on a
multi-binder arm — or the projected-tuple rewrite mixed up which binder feeds
which slot per branch — the swapped-role branches would corrupt.

Result: PASS ×3 wasm + rust + rust-async, correct hand-modeled outputs
(90028180201025 @ n=10 / 41029032240020 @ n=0; the two seeds route
[0,1,2,1] vs [1,2,0,0] through the residues — full branch coverage swapped).

The collapse holds for chained binders + per-branch role swaps. Promotable
as a pass-pin companion to the tpwJ fix-pins (batch-295+).

Trunk f00670782.
