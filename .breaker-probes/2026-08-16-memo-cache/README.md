# cch1 — one-slot memo cache with Option state (2026-08-16, tick 1627)

Attack: a TRIPLE-NESTED match in the arm — tuple destructure, then Option
(Some/None), then inner-tuple destructure — with the MISS rebuild constructing
`(Some (tuple k (+ (* k k) (% n 3))))` (an Option-of-tuple allocation whose
payload compound also appears in the answer). The Some and None arms share an
identical miss body (duplicated deliberately — a CSE across match arms that
mis-shared the constructed Some would corrupt). First Option-typed handler
state in my probe series (corpus has oc1 Option-of-tuple accumulator; this
adds hit/miss counters + occupancy read + seed-swapped branch pattern).

Differential: bias re-keys the THIRD lookup: n=0 re-hits the hot key (3-hit
run: rows 90,91,91,91 stats 311) where n=10 probes a cold key (miss on 4,
evicting 3's entry... actually re-caching 4 then re-missing... no: rows
100,101,170,100 — the 4th get MISSES because the cold probe evicted key 3 —
wait, it answers 100 = (9+1)*10 = re-computed miss. Yes: the one-slot evict
means n=10's fourth get re-misses the ORIGINAL key. stats 131 vs 311 (hits
and misses swap roles).

Hand model: n=10 → 1000101017001000131; n=0 → 900091009100910311 (base-10000).

Pass ×3 wasm + rust + rust-async on trunk 6106503ee (operand-position E0308
twin landed this tick — v-rb closing the fz class face by face).
