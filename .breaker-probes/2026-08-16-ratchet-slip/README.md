# rch1 — ratchet with phase-keyed slip (2026-08-16, tick 1600)

Attack: sticky-vs-transient state interaction. A 3-tuple state (pos, phase, slips)
where the SLIP branch derives BOTH its answer and its next-pos from the same
compound `(- pos (% pos 3))` — a shared subexpression across the resume value and
the state rebuild, in the F24-adjacent shape but at a safe envelope (2 branches,
6 dispatches, 3-tuple).

Differential: seed picks WHICH phase slips (n%4). n=10 → phase 2 slips once at
pos 53→51... wait, answers dropped-pos+50. n=0 → phase 0 slips TWICE (first and
fifth click, since phase wraps mod 4 over five clicks); the second slip drops
pos 12→12 (already a multiple of 3) — invisible in position, visible ONLY in the
+50 answer offset and the slip counter. That "invisible slip" is the pin: an
optimizer that folds the slip branch when `pos % 3 == 0` (no positional change)
must still produce the offset answer and bump the counter.

Hand model (python, banked in transcript):
- n=10: rows [2,5,53,8,10] + slips 1 → 20553081001
- n=0:  rows [50,3,7,12,62] + slips 2 → 500307126202

Pass ×3 wasm, ×1 rust, ×1 rust-async on trunk 720e2fa97.
