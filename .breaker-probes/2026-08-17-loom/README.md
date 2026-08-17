# lom1 — loom with phase-shifted rows (2026-08-17, tick 1677)

Attack: a DIRECTION-FLIP state machine where row completion rides the flip
(leftward return = row+1 + possible pattern row; rightward pass = plain tag)
— the 3-leaf weave arm's completion branch splits on `(% (+ row 1) 2)` with
the incremented row in both the answer and rebuild. The mend op UNDOES a
row (floored) — a decrement against weave's increment, with the floor branch
resuming a partial rebuild.

Differential: starting shuttle direction phase-shifts which weaves complete
rows: n=10 (dir 1) completes on weaves 1,3 — weave 3 completes row 2, THE
PATTERN ROW fires (720); n=0 (dir 0) completes only on weave 2 (row 1, odd
— pattern dark). Mend unpicks row 2→1 vs row 1→0 (floor visible in tag).

Iteration notes: first draft's 4-leaf arm at 5 dispatches instruction-
declined; the 4-dispatch trim still declined (the extra leaf in the
completion path counts); final form re-shaped the arm to 3 leaves
(completion-split + plain-pass) at 4 dispatches — passes. Envelope: 4-leaf
fence sits at ≤3 dispatches (sil), 3-leaf at 4+ (this).

Hand model: n=10 → 10011720011101; n=0 → 1010011010011 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 0db236a9d.
