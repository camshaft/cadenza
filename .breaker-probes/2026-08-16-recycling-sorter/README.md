# rcy1 — recycling sorter with contamination eviction (2026-08-16, tick 1651)

Attack: a 3-residue dispatch whose contaminant leaf fans into a further 3-way
(evict-paper / evict-glass / evict-nothing) via the boolean-AND-as-nested-if
`(if (>= p g) (> p 0) false)` — the tie-goes-paper rule from scl1's family
applied to an EVICTION target choice. All three eviction branches share the
identical answer `(+ 900 (+ c 1))` while diverging ONLY in the rebuild (a
same-answer/different-state triple — CSE on answers must not merge states).

Differential: the seed shifts item #2's code (5→glass vs 6→... 5+1=6 paper?
no: n=10 gives 6 → residue 0 → PAPER; n=0 gives 5 → residue 2 → CONTAMINANT.
n=0's early contaminant evicts the lone paper; its second contaminant (8)
then evicts GLASS — both eviction targets exercised; n=10 evicts paper once.
Audit 111 vs 002 (n=0 ends empty-binned).

Hand model: n=10 → 12011021901111; n=0 → 12901011902002 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 85bb67940.
