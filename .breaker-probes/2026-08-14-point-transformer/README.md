# pts1 — 2D point under quarter-turn rotations (2026-08-14, tick 1482)

3-op handler over (x,y): `rot` maps (x,y)→(y,-x) answering old-y*10 plus a
sign tag of the NEW y; `mv` translates answering the Manhattan norm (iabs
def); `quad` reads the quadrant via nested sign branches. Seeds trace
different orbits (n=10 crosses three quadrants; n=0 stays near the axes;
both end quadrant 1 but through different paths — intermediate rows differ).

First gate run FAILED (all 3 backends agreed against the .sexp): my sign-tag
branch was INVERTED relative to the python model ((< (- 0 x) 0) 1 -1) vs
model's -x<0 → -1. The differential caught the transcription slip; fixed to
(-1 1) and green ×3. Negative composite answers (-121/-21 rows) ride the
digit packing.

PASS ×3 wasm. **Pool — completes the vig1/ffs1/pts1 trio (next-next send).**
