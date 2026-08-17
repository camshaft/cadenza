# cdl1 — candle dipping rig (2026-08-17, tick 1697)

Attack: an INVERSE-PROPORTIONAL growth compound `(- 6 (/ th 2))` — the growth
reads the field it grows (rnk1's feedback family but CONTINUOUS: the rate
shrinks as the field rises, vs rnk1's threshold flip) — floored at 1 via an
if-pair where the floor branch's answer hard-codes the 1 (constant 10-row +
% tail). The growth compound appears x4 in the live branch (test, answer x2
via the nested new-thickness mod, rebuild).

Differential: bare wick (0) vs primed (4): n=0 gains 6 then 3 (66, 39 — the
fast-then-slow curve); n=10 gains 4 then 2 (48, 20 — already slowed); both
converge to the SAME drip row (701) via different paths — reads 621 vs 721
(final thickness 6 vs 7).

Hand model: n=10 → 488080207010721; n=0 → 668060397010621 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk c6fa89785.
