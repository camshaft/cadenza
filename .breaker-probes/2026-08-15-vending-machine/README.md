# vnd1 — vending machine with a change float (2026-08-15, tick 1552)

(credit, float) state: `insert` accumulates; `buy` vends only when credit
covers the price AND the float covers the change — answering the change,
growing the float by price−change, zeroing credit — with two DISTINCT
refusal codes (plain −shortfall for insufficient credit; −shortfall−50 for
insufficient float, credit KEPT).

Seed float (n%4: 2 vs 0): n=10 completes two sales (change 2 then 3, float
compounding 2→7→11); n=0 never completes one — every buy refuses, credit
accumulates to 19, and the last two rows show the same float-refusal twice
(-62, -62: state truly frozen). One run's machine works; the other's is
bricked by an empty float — end-to-end behavioral divergence.

3-branch arm, 2-tuple — envelope-safe. PASS ×3. **Pool (with gsc1; +1 fills
the 10th trio).**
