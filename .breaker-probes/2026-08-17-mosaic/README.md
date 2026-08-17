# msc1 — mosaic bench with tile nipping (2026-08-17, tick 1707)

Attack: the PARTIAL branch reads the field it zeroes TWICE with different
signs — `(- k tiles)` (shortfall, answer + waste) and `(+ laid tiles)` (the
remnant credited) — before storing 0 (frg1's read-before-clobber doubled:
two reads, opposite roles, one clobber). The covered branch's answer packs
POST-deduct values on both sides. Nip's chip `(/ k 3)` lands in the answer's
mod and the waste rebuild.

Differential: tray 8 vs 5: n=10 covers every course (rows 44,102,113,140 —
read 142: tray 1, laid 14, waste 2); n=0 runs SHORT on the final course
(803 — shortfall 3... rows [41,72,110,803], read 115: tray 0... trust model:
115). Waste 2 vs 5 splits chips-only vs chips+shortfall.

Hand model: n=10 → 441021131400142; n=0 → 410721108030115 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 0657b816d.
