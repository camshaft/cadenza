# sil ladder — grain silo pair with auger (2026-08-17, tick 1676)

Attack: TWO 4-leaf arms in one protocol — dump routes to the EMPTIER silo
(tie-to-A, the scl1 rule over a different pair) with per-target cap+spill
leaves; auger's move quantity is an inline min whose result feeds A's
decrement, B's cap test, AND the answer. Cross-op: the auger changes which
silo is emptier, re-routing the next dump.

## Envelope
- sil1 (4 dispatches x two 4-leaf arms): scratch-locals clean decline —
  two heavy arms compound (the lnd/cnv law: leaves x dispatches, now
  summed across ARMS).
- sil2 (3 dispatches): PASSES x3 all backends. Differential: pre-filled A
  (4 vs 0) → every dump targets the other silo; runs never re-align
  ([270,30,170] read 800 vs [170,30,290] read 490). Spill leaves are DARK
  on both seeds (no overflow at these amounts) — the routing + auger-cap
  are the pins; spill coverage lives in aqm1/stm1's clamp family.

Hand model: n=10 → 270030170800; n=0 → 170030290490 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 0db236a9d.
