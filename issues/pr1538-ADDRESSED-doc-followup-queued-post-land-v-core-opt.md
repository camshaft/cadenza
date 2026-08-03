# PR #1538 review comment — implementation/seed/crates/rcdzc/src/lower.rs (v-core-opt)

Mirrored from https://github.com/camshaft/cadenza/pull/1538 (PR: "[v-core-opt] 77886c353").
The `lower_float_of` operand-width-demote fix (adv-61 fold-precision class, width-conversion face).
Copilot APPROVED the PR (behavioral change small + regression-tested); one doc-drift point remains.

## Function-level doc comment for `lower_float_of` still says "round at target width" only (Copilot, lower.rs:17301) — doc
> The function-level doc comment for `lower_float_of` (just above this block) still describes constant
> folding as rounding only at the *target* width, but the implementation now also reads/demotes at the
> *operand* width first via `const_float_bits_at_operand_width`. Please update the doc comment so it
> reflects the two-step (read-at-operand-width, then round-at-target-width) fold.

VERIFIED against the current tree: the inline block comments WERE updated to describe the
read-at-operand-width step, but the `///` function doc (lower.rs:17274-17278) still reads "FOLD a
constant float by rounding the exact `Decimal` at the TARGET width … a same-width or widening
conversion is exact" with no mention of the operand-width demote-first step. Doc-only; update the
`///` block to match the two-step fold the body now performs.
