# hrn1 — Horner evaluator with mid-stream base swap (2026-08-14, tick 1489)

(acc, base) state: `feed` folds acc*x+c answering the running value — a
digit-append in base x — and `swapx` replaces the base answering the OLD one.
Seed shapes the initial base ((n%4)+2: 4 vs 2), so pre-swap accumulations
diverge (13 vs 7); the swap answer itself is the seed-differential (4 vs 2);
post-swap the SAME coefficients ride on top of different accumulators
(1342 vs 742 packed in base 10).

The packing answers grow to 4 digits (1342) inside the *100 packing — rows
overlap intentionally; the hand model packs the same way so the pin is exact.

PASS ×3 wasm. **Pool (with qrm1/lru1/pas1 → next trio is qrm1/lru1/pas1,
hrn1 seeds the one after).**
