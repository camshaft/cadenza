# cdr1 — cider press (2026-08-17, tick 1713)

Attack: a CONSERVATION split — the press divides the hopper into juice
`(/ (* hop 2) 3)` and pomace `(- hop (/ (* hop 2) 3))` where the two parts
must sum to the emptied hopper (mul-then-div + subtract-of-the-div: the yield
compound appears x4 — sentinel test, answer, both rebuild fields — and the
pomace derivation embeds it). The dry press is the sentinel-guard over the
same compound (spn1's family with a 2/3 ratio instead of half).

Differential: hopper 7 vs 2: n=10's first press splits 4/3 (43); n=0's splits
1/1 (11) — the integer 2/3 ratio lands differently on every size (7→4+3,
2→1+1, 4→2+2), reads 605 vs 303 with pomace trails 5 vs 3.

Hand model: n=10 → 430440259050605; n=0 → 110440239030303 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 141665bdd.
