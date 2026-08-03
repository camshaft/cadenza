# Baseline dedup heal — fleet-wide baseline-no-dup-titles RED

v-memory-safety flagged (note 21791): all 3 .gate-baseline* on trunk had duplicate case-title rows
(wasm 196, rust 327, rust-async 190) → baseline-no-dup-titles hard-error in xtask check → gate --check
RED fleet-wide. Baseline is corpus-bugfix's lane.

## Fixed (MR c095c0e14, QUEUED)
- Regenerated all 3 via `cargo xtask gate --target <t> --save` → one line per case at true verdict.
  5525 cases each, 0 fail (wasm 5499p/26t, rust 5470p/55t, rust-async 5422p/103t).
- Pure dedup: case-title SET byte-identical to HEAD (verified), only dup rows dropped + re-sorted.
- 4 conflicting rust dups (effects/host + @param) → pass (real current rust emit).
- gate --check OK ×3, 0 regressions, no .sexp touched.
- Unblocks every held MR fleet-wide.
