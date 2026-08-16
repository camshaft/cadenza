# wnd1 — wind-chill stepper with an extreme clamp (2026-08-15, tick 1580)

(temperature, extremes) state: `gust` answers temp − 2·wind CLAMPED at −30
(clamp counted); `warm` raises the temperature; `ext` reads the count.
Note the wind is NOT stored — each gust recomputes the chill from its
argument (stateless-wind design keeps the tuple at 2).

Cold seed (−5) drives the 16-gust past the clamp (−31 → −30, extreme 1)
where the warm seed (5) reads −21 unclamped — and both packed totals ride
deep negative with EVERY gust row negative on the cold run. The warm rows
(11/31 vs 1/21) carry the constant temperature offset while the chill rows
diverge nonlinearly (the ×2 wind multiplier).

PASS ×3. **Pool — fills lgt1/mrt1/wnd1 (11th trio ready).**
