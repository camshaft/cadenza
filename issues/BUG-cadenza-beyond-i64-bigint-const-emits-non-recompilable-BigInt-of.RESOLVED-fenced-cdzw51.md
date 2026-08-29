# BUG: cadenza backend emits beyond-Int64 BigInt const as non-re-compilable (BigInt.of <beyond-i64>)

Found tick 432/433 (2026-08-28) via the dual-path VALUE oracle probing #4875 (cadenza wrapper-const emit). ROUTED to v-cadenza-backend.

## Defect
#4875 re-emits `ConstInt @ Ty::BigInt` as `(BigInt.of n)`. But `BigInt.of` WIDENS a fixed-size Int64, so for |n| > Int64.MAX the re-emitted `.ast` fails to re-compile: CDZ0201 "integer literal out of range for Int64 … write the literal directly as a BigInt with (: … BigInt) instead of (BigInt.of …)". The cadenza round-trip (sexp→cadenza(.ast)→wasm) thus FAILS on any beyond-i64 BigInt constant.

## Precise boundary (dual-path)
- `(def (v) (: 9223372036854775807 BigInt)) (export v)` → hop OK (Int64.MAX)
- `(def (v) (: 9223372036854775808 BigInt)) (export v)` → hop FAILS (MAX+1)
- in-range `(BigInt.of 42)`, Symbol `#"s"`, `Rational.of` → all OK. Only beyond-i64 BigInt breaks.

## Fix (per the CDZ0201 message)
Re-emit a beyond-Int64 BigInt constant as the direct literal `(: n BigInt)`, not `(BigInt.of n)`. In-range may stay `(BigInt.of n)`.

## Impact
corpus-cadenza REDS on any B0 leaf case whose constant is a beyond-i64 BigInt (the residual of the tick-418 wrapper-const class). breaker did NOT touch compiler code; will author a beyond-i64-BigInt cadenza witness once fixed.
