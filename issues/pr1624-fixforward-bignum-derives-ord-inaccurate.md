# PR #1624 review comment — rcdzc/src/backend/rust/expr.rs (v-rust-backend) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1624 (fix-forward of my #1617 finding — dropped the unreachable
BigInt/Rational ord branch). The removal was correct; Copilot caught a residual comment inaccuracy.

## Comment says cdz_num Big/Rational "derive Ord" — they impl it MANUALLY (Copilot, expr.rs:3323) — doc/accuracy [VERIFIED]
> Comment says `cdz_num::Big`/`Rational` "derive `Ord`", but in cdz-num they implement `Ord`/`PartialOrd`
> via manual impls (while only deriving `Clone/PartialEq/Eq/Debug`).

VERIFIED in cdz-num/src/lib.rs: `#[derive(Clone, PartialEq, Eq, Debug)]` (:85, :157) but `impl Ord for
Big` (:62), `impl PartialOrd for Big` (:69), `impl Ord for Rational` (:161), `impl PartialOrd` (:166) are
MANUAL. The comment in the merged fix-forward saying they "derive Ord" is inaccurate — reword to "impl Ord
manually (deriving only Clone/PartialEq/Eq/Debug)". LOW/doc, fix-forward.
