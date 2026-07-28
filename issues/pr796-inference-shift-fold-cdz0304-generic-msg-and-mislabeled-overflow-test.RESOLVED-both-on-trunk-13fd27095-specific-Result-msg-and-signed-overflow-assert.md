# PR#796 review comments — solved-width shift/bitwise fold: generic CDZ0304 message + mislabeled "overflow" test

Mirrored from GitHub PR review comments (Copilot), ids `3632909224`, `3632909280`.
PR: https://github.com/camshaft/cadenza/pull/796 (batch-staging; fixes belong on trunk)
Locations: `implementation/seed/crates/rcdzc/src/lower.rs:18433`, `implementation/seed/crates/rcdzc/src/tests.rs:25586`.
Both landed with the solved-width shift/bitwise fold (`b2197d097`, "4th/final UInt64 fold slice"; the
fold family is v-inference's `edc5e15bf`).

## Comments (verbatim)

- (id 3632909224, lower.rs:18433) "The CDZ0304 diagnostic produced by the new solved-width
  shift/bitwise fold is much less specific than the existing `fold_arith` path (it always emits a
  generic 'count out of range or overflow' message). Since the fold has enough information here to
  distinguish an out-of-range shift count from a width overflow, it should report a more actionable
  message (including the offending count and the bit width) to keep constant-trap errors consistent
  and debuggable."
- (id 3632909280, tests.rs:25586) "This block's comment says it is asserting that a signed `<<`
  overflow is rejected, but the assertion actually checks a non-overflowing shift (`-8 << 1`) still
  folds. This is misleading and also leaves the overflow behavior untested in this regression."

## Liaison verification (CONFIRMED on trunk)

1. lower.rs ~18428-18437 (`Prim::Shl` fold at solved width): distinguishes `count >= width` (out-of-range
   count → None) from `checked_shl` / `s == (s & mask)` (width overflow → None), but both surface the
   same generic CDZ0304. It HAS the count + width in hand → could message "shift count N ≥ width W" vs
   "shift result overflows width W". Diagnostic-quality (matches fold_arith's more actionable style).
2. tests.rs:25585-25587: comment "A genuine signed i64 `<<` overflow is still rejected" but the assert
   is `reject_code("(<< (- 0 8) 1)").is_none()` with msg "a small signed `<<` that fits still folds" —
   it tests a NON-overflowing signed shift (`-8 << 1 = -16`) folds. So the comment claims overflow-
   rejection while the code tests non-overflow-folds; the signed-`<<`-overflow case is UNTESTED here.
   Fix: either correct the comment to match (it tests fold-not-reject), OR add the actual overflow
   assertion (a signed `<<` that overflows Int64 → `reject_code(...) == Some("CDZ0304")`). Test-quality.

Owner: v-inference (owns the shift/bitwise fold family — `edc5e15bf` / `b2197d097`; emit/fold-type
selection lane). Routed as a note. Both minor (diagnostic + test-quality), no runtime miscompile.
