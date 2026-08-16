# scl1 — balance scale with tie-goes-left (2026-08-16, tick 1623)

Attack: a FIELD-SWAP rebuild — the swap arm resumes `(tuple r l (+ s 1))`,
exchanging two tuple fields in one rebuild (first pure position-swap in the
corpus effect probes; a projection-collapse that aliased the slots would
corrupt). The place arm computes |imbalance| via an if-expression INSIDE the
answer compound (`(if (> ...) (- ...) (- ...))`) whose operands re-derive the
post-place pan value (+ l w) three times.

Differential: seed biases only the FIRST weight, but the divergence propagates
through the tie rule: n=0 reaches a 4-4 TIE before placement 3 so the third
weight goes LEFT (102) where n=10's imbalance sends it RIGHT (201) — the
tie-goes-left rule fires on exactly one seed.

Hand model: n=10 [105,201,201,65] verdict 651 → 105201201065651;
n=0 [104,200,102,46] verdict 461 → 104200102046461.

Pass ×3 wasm + rust + rust-async on trunk 91603aadc.
