# PR#756 review comment — rust unusual-width `Mul` uses wrapping_mul on storage type, can wrap past 2^storage and defeat the range-check (MISCOMPILE)

Mirrored from GitHub PR review comment (Copilot), id `3625771358`.
PR: https://github.com/camshaft/cadenza/pull/756 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/backend/rust/expr.rs:3295` (the unusual-width Add/Sub/Mul arm)

## Comment (verbatim)

> For unusual-width integer multiplication, emitting `wrapping_mul` on the storage type can overflow
> the storage width (e.g. UInt48 values like 2^32 * 2^32) and wrap back into the unusual-width range,
> causing the subsequent range-check to incorrectly PASS and silently miscompile instead of trapping
> overflow. The implementation needs to detect storage-width overflow for `Mul` (e.g. use
> `overflowing_mul`/`checked_mul` plus an explicit trap, or compute in a wider intermediate like
> i128/u128) before applying the width-N bounds check.

## Liaison verification (CONFIRMED — real, though reachability is currently guarded)

The arm (expr.rs ~3286-3300) handles unusual widths (1..=64 minus 8/16/32/64) for Add/Sub/Mul by:
1. emitting the NATIVE wrapping op on the STORAGE primitive (`wrapping_add`/`wrapping_sub`/`wrapping_mul`),
2. then range-checking the result against the TYPE's own `[min_N, max_N]`, panicking "integer overflow"
   if outside.

The soundness argument (in the arm's comment) is: "the storage type never wraps at `2^machine` for
in-range operands". That holds for **Add/Sub** — two in-range UInt48 values sum to < 2^49 ≪ 2^64, so
`wrapping_add` on u64 equals the true result and the range-check is sound.

It does NOT hold for **Mul**: two ~2^47 UInt48 values multiply to ~2^94, far beyond u64's 2^64, so
`wrapping_mul` wraps modulo 2^64 and can land BACK inside `[0, 2^48-1]`. Concrete: UInt48 `2^32 *
2^32 = 2^64` → `wrapping_mul` on u64 = `0` → `0` passes the `[0, 2^48-1]` check → NO trap, silent wrong
result. Should trap "integer overflow".

Fix (per Copilot): for `Mul`, compute in a WIDER intermediate (u128/i128) or use
`checked_mul`/`overflowing_mul` on the storage type + explicit overflow trap, BEFORE the width-N bounds
check. (Add/Sub can stay on the storage wrapping op — they can't exceed 2^storage for in-range unusual
operands.)

Reachability caveat: the arm's OWN comment says "no corpus case runs it — the only unusual-width `+`
is a compile-time CDZ0304 reject … this guard is what keeps a FUTURE runtime unusual-width arith from
miscompiling". So the unsound path may not be reachable via any current corpus program — but it is live
code that IS unsound for Mul, and it's exactly the "defense-in-depth" path meant to be correct-by-
construction. Worth fixing now so a future runtime unusual-width Mul doesn't silently miscompile.

Cross-backend note: the comment calls this "the rust twin of the wasm narrow-width `emit_range_check`
(select.rs)". The WASM backend's unusual-width Mul path (select.rs `emit_range_check` sites) may share
the SAME gap — v-rust-backend should check the wasm twin too (or flag v-inference, who owns the wasm
select emit).

Owner: v-rust-backend (integrated `0c7797b79` "runtime arithmetic on an unusual integer width
range-checks at its OWN width, not the storage width"). Routed as a note flagged
CONFIRMED-MISCOMPILE (latent/guarded reachability).
