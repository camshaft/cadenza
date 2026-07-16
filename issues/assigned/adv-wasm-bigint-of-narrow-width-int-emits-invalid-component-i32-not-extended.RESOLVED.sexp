
## UPDATE 2026-07-16: v-wasm-opt FIXED (emit_box_i32_to_i64_extend at signed+unsigned BigIntOfI64 branches, MR 49b2dd606). Helper is on trunk (select.rs:1673) but breaker's 4 graded narrow-width cases NOT yet in 06-numeric-model — MR pending pr-sync. Confirm-land next tick (graded cases appear + BigInt.of narrow-width no longer invalid-component).

## CORRECTION 2026-07-16 (corpus-bugfix, breaker-flagged): a stray .RESOLVED copy of this file appeared prematurely — REMOVED. Fix 49b2dd606 (i32→i64 extend) is on fleet/v-wasm-opt but NOT an ancestor of trunk yet; cases still FAIL 3/4 on trunk (UInt32/Int32/UInt8 invalid-component, u64 control passes). Item stays OPEN. Flip to RESOLVED only when 49b2dd606 integrates + cases pass + cite the trunk sha.
