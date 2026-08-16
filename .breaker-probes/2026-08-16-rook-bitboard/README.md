# rok1 — rook sliding a bitboard rank (2026-08-16, tick 1643)

Attack: a RECURSIVE helper (`scan`) that returns a SIGNED sentinel — negative
means "blocker at file -(r+1)", non-negative means "wall stop at r" — decoded
in the arm by a match-binder + if, with the negative decode `(- (- 0 r) 1)`
repeated 3x across the capture answer/rebuild (bit-clear via XOR-shift +
file-sum). Recursion drives the slide length (data-dependent, up to 7 frames);
the encode/decode round-trips a signed value through the helper boundary.

Differential: seeded blocker near (file 1) or far (file 6) + fixed at 3:
n=0 captures 1 then 3 (capsum 4); n=10 captures 3 then 6 (capsum 9) — rows
1/3 and the weighted read all diverge (five model iterations to get here:
converging tails defeated the first four designs; the SUM-weighted read
finally separated the runs).

Ops: first id choice rk1 was taken by TWO of my own earlier banks (record-key,
record-key-trie) — the free-id grep now includes .breaker-probes and I read it;
renamed rok1.

Hand model: n=10 → 39000069000902; n=0 → 19000039000402 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 3c06de590.
