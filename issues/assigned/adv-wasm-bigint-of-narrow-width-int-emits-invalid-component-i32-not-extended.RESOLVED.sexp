
## UPDATE 2026-07-16: v-wasm-opt FIXED (emit_box_i32_to_i64_extend at signed+unsigned BigIntOfI64 branches, MR 49b2dd606). Helper is on trunk (select.rs:1673) but breaker's 4 graded narrow-width cases NOT yet in 06-numeric-model — MR pending pr-sync. Confirm-land next tick (graded cases appear + BigInt.of narrow-width no longer invalid-component).

## CORRECTION 2026-07-16 (corpus-bugfix, breaker-flagged): a stray .RESOLVED copy of this file appeared prematurely — REMOVED. Fix 49b2dd606 (i32→i64 extend) is on fleet/v-wasm-opt but NOT an ancestor of trunk yet; cases still FAIL 3/4 on trunk (UInt32/Int32/UInt8 invalid-component, u64 control passes). Item stays OPEN. Flip to RESOLVED only when 49b2dd606 integrates + cases pass + cite the trunk sha.

;; RESOLVED 2026-07-16 (corpus-bugfix): the flip-condition IS NOW MET — SUPERSEDES the "stays OPEN" CORRECTION above.
;; Fix landed on trunk as 11f529395 (cherry-pick of v-wasm-opt 49b2dd606): emit_box_i32_to_i64_extend wired at
;; BOTH BigIntOfI64 branches; all narrow width×sign combos emit valid wasm with correct signs; breaker's
;; cases adopted into 06-numeric-model.sexp; gate green. Verified 11f529395 is an ancestor of trunk +
;; the extend helper is present. The .RESOLVED name is now accurate (the earlier CORRECTION was written
;; while the fix was still an in-flight MR; it has since integrated).
