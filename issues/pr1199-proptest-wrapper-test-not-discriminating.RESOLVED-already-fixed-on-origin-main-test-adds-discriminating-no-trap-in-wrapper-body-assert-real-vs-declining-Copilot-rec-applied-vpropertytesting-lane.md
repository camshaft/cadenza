# PR #1199 review comment — rcdzc/src/proptest_gen.rs (v-property-testing)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1199
(PR: "cand: v-property-testing — c0672c0b3").

## Test doesn't distinguish a real generator wrapper from a declining one (Copilot, proptest_gen.rs:2524) — test-coverage
> This test currently doesn't actually distinguish a REAL generator wrapper from the DECLINING
> wrapper. A declining wrapper is also nullary and also replaces the original def name, so the
> current assertions would have passed even before the classify_sum fix (when mixed/nullary sums
> declined). To make the test meaningful, assert something that differs between the two wrappers
> (e.g., that the wrapper body does not contain a `trap` form).

The test can't fail for the bug it's meant to pin: both the real and declining wrappers are nullary
and rename the def, so the current assertions passed pre-fix too. Add a discriminating assertion —
e.g. the real wrapper body contains NO `trap` form (the declining wrapper does) — so the test
actually guards the classify_sum fix.
