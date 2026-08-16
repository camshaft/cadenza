# seg1 — seven-segment display driver (2026-08-16, tick 1630)

Attack: TWO recursive-helper calls per arm — `(pop (segmask d) 0)` and
`(pop (^ (segmask d) mask) 0)` — each appearing in BOTH the answer and the
rebuild (4 recursive calls per dispatch), where `segmask` is itself a 5-way
nested-if constant ladder called 4x per dispatch. Interesting envelope
datapoint: PASSES at 3 dispatches despite heavy shared-compound repetition —
recursive helper CALLS apparently don't mint scratch slots the way inline
compound trees do (the call is one instruction; the fence law is about
inline recompute width).

Differential: seed picks the middle digit (2 vs 0): digit 0 lights 6 segments
vs digit 2's 5, so the flip counts diverge (rows 53/43 vs 55/45) and stats
diverge (1184 vs 1224); first row (digit 1) agrees.

Hand model: n=10 → 220530431184; n=0 → 220550451224 (mixed base:
3 rows base-1000, stats tail base-10000 — first-draft base-1000 overflowed
the 4-digit stats row; bounds-check caught it).

Pass ×3 wasm + rust + rust-async on trunk 931c11dd3.
