# 2026-08-13 prepend-rope accumulator (tick 1385)

- `psr1.sexp` — PREPEND accumulation: `(String.concat p s)` puts each new piece
  in FRONT, so the oldest piece rides at the END of the rope; the closing check
  slices the tail window and confirms it equals the first-pushed piece. The
  handle-result-as-seed angle was coverage-killed (si1/si2/sc1 matrix at
  14c:4631). All landed string-state pins accumulate by APPEND (concat s piece);
  prepend builds the rope's spine in the opposite association — left-deep vs
  right-deep — and the tail-slice read crosses the deepest node. Seed varies
  the first piece's WIDTH (1 vs 3 bytes) so lengths and the slice offset both
  shift. PASS ×3 (13051/35071).
