# dic ladder — LCG die with hot-streak multiplier (2026-08-16, tick 1639)

Attack: the face compound `(+ (% (% (+ (* s 7) 5) 31) 6) 1)` — an LCG advance
nested inside a double-mod — repeats 6x across the 2-branch arm (condition,
both answers, both rebuilds via score), and the streak branch's score term
multiplies it by the deepened streak. Streak-dependent scoring = the multiplier
deepens per consecutive high face.

## Envelope
- dic1 (5 rolls): scratch-locals clean decline (6x compound + LCG nesting —
  consistent with the compound-count law).
- dic2 (3 rolls): PASSES ×3 wasm + rust + rust-async. Differential: n=10 opens
  with a DOUBLE streak (616, 628 = streak 2) then breaks; n=0's second roll
  breaks immediately (616, 400). Tally 210 vs 110.

Model notes: first LCG params (5,7,36) gave NO streak on either seed — face
coverage must be CHECKED against the model, not assumed (searched 4 param sets
for double-streak coverage). And the sed-derived dic2 initially carried a stale
expected (616628301131 vs modeled ...210) — caught by the gate, fixed from the
model. Sed-derives need the model re-run EVERY time (2nd occurrence of this
slip; irg3 was the first).

Pass ×3 on trunk 4c75635d9. dic1 held for (b).
