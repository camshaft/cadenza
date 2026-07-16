
## UPDATE 2026-07-16: v-wasm-opt FIXED (emit_box_i32_to_i64_extend at signed+unsigned BigIntOfI64 branches, MR 49b2dd606). Helper is on trunk (select.rs:1673) but breaker's 4 graded narrow-width cases NOT yet in 06-numeric-model — MR pending pr-sync. Confirm-land next tick (graded cases appear + BigInt.of narrow-width no longer invalid-component).
