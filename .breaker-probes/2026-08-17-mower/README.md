# mow1 — mower with grass-catcher (2026-08-17, tick 1703)

Attack: a min-clamp CROSSED with a threshold — the mow arm splits on the
clamp side (grass < 5) THEN on the catcher overflow, giving 4 leaves where
the cut amount is the VARIABLE grass on one side and the CONSTANT 5 on the
other (each side's overflow test uses its own cut: `(+ cat grass)` vs
`(+ cat 5)`). The auto-empty rebuild zeroes catcher while the answer reads
the incremented empties — chr1's asymmetric clamp fused with ftn1's
reset-to-constant.

Differential: lawn 7 vs 3: n=10 cuts 5 (constant side, 55), grows, cuts 5
again overflowing (715 — auto-empty #1), final short cut 3 (33); n=0 cuts 3
(variable side, 33), grows, cuts 5 (58), final cut overflows (711). The
auto-empty lands on pass 2 vs pass 3; reads 31 vs 1.

Hand model: n=10 → 550867150330031; n=0 → 330660587110001 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 86ae0a4bc.
