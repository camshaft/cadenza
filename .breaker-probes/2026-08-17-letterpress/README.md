# tps1 — letterpress galley (2026-08-17, tick 1712)

Attack: an OFF-BY-ONE-SHAPED fit test — the word costs `(+ w 1)` (width plus
space) in the test AND the taken rebuild, but the BREAK branch stores the
BARE width w (no space on a fresh line) — the same argument enters state two
ways depending on the branch. Justify computes the gap `(- 12 lw)` twice
(answer + respacing accumulation) then stores the CONSTANT 12 — a gap-then-
saturate where the answer reads the pre-saturation value. The zero-gap
justify (n=0: lw already 12) exercises the degenerate gap=0 row.

Differential: headline stub 5 vs 0: n=10 breaks twice (716, 723 — the stub
crowds every line); n=0 sets flush (54, 126), justifies with gap ZERO (0 —
the degenerate row), breaks once. Reads 236 vs 130.

Hand model: n=10 → 1047160667230236; n=0 → 541260007130130 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 141665bdd.
