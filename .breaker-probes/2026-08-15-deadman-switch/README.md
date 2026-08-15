# dth1 — dead-man's switch with an absorbing latch (2026-08-15, tick 1555)

(misses, uptime, latch) 3-tuple: `beat` resets misses answering the uptime
tick; `poll` counts a miss until the seed-shaped threshold ((n%3)+2: 3 vs 2)
LATCHES — and once latched EVERY answer is -9 forever, beats included (the
latch is absorbing). The lower threshold latches six rows early; the long -9
tail on n=0 pins absorption through both ops.

Envelope note: 3-tuple × 2-branch cheap arms at 8 dispatches PASSES — the
latch guard reads one field and each branch writes disjoint fields; compare
bnf (3-tuple + range guard ACROSS fields declined). More evidence the
cross-field guard structure, not width alone, is the frontier.

PASS ×3. **Pool (with tdr1; +1 fills the 11th trio).**
