# PR #1869 review comments — rcdzc/src/tests.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1869 (pin literal range-check reaching a narrow-via-call-chain width).

## Both "TRANSITIVE" cases annotate `f`'s param UInt8 → the narrowing is DIRECT, not transitive-through-the-chain (Copilot, tests.rs:18349 + :18376) — test-precision [VERIFIED plausible]
> The "TRANSITIVE through a two-call chain" REJECT case still has explicit `(: x UInt8)` on `f`, so the
> out-of-range literal is rejected DIRECTLY at `(main)->f`, not via transitive narrowing through `g`. The
> "IN-RANGE transitive" clean case likewise annotates `f`'s param — so it'd pass even if inference failed
> to thread the width across the chain. For both, leave `f` unannotated and narrow only by passing x into
> `g`'s UInt8 parameter, so the test actually depends on the transitive propagation.
Same "test doesn't exercise the claim" class as #1652/#1662/#1688: both cases name themselves TRANSITIVE
but the explicit `(: x UInt8)` on `f` short-circuits the chain — the width comes DIRECTLY from f's own
annotation, so the test passes regardless of whether inference threads the width through g. To pin the
actual transitive path, drop f's annotation and let g's UInt8 param be the only narrowing source. MED
test-precision (a green test that doesn't guard the transitive propagation it's for). Fix-forward.
