# vtc1 — vote-to-close quorum (2026-08-15, tick 1577)

(count, closed) state: `second` registers answering the count until the
seed-shaped threshold ((n%3)+2: 3 vs 2) CLOSES the motion — answering
100+count, with every later op answering -100 (absorbing close, both ops);
`withdraw` decrements while open, clamping at zero. The withdraw-then-reseCond
sequence makes the count SAW rather than climb (1,0,1,2,…), so the lower
threshold closes one second earlier and its tail is closed sentinels.

vs dth1 (absorbing latch on failure): vtc1 latches on SUCCESS with a
saw-tooth approach — the complementary absorbing face.

PASS ×3. **Pool — fills wrd1/spr1/vtc1 (10th trio ready).**
