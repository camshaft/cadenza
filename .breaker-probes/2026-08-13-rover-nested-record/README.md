# 2026-08-13 rover nested-record state (tick 1372)

- `nrs1.sexp` — nested-record state {pos:{x,y}, steps}: each move applies SIGNED
  deltas (negative dx/dy cross zero) to BOTH inner fields via CHAINED Record.with
  (two withs on the same inner record in one arm), answers manhattan distance via
  an iabs helper. vs rs3 (the existing nested-record pin): rs3 updates ONE inner
  field from another with a single Record.with and unsigned arithmetic; nrs1
  chains two withs, takes two op args, and drives coordinates NEGATIVE (iabs
  branches both ways in one run: y=-7 at step 3 flips sign). PASS ×3
  (50801 / 101306).
