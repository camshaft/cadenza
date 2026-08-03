# PR #1070 review comment — rcdzc/src/tests.rs (v-wasm-opt)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1070
(PR: "cand: v-wasm-opt — guarded dead-arm elimination coverage").

## Test asserts constants but not guard removal (Copilot, tests.rs:48696) — test-coverage
> The comment says the dead guarded arm's guard `(> x 0)` is "never emitted", but the assertions
> only check for `ConstI64(100)` and `ConstI64(111)`. This doesn't actually prove the guard was
> eliminated (the guard code could still be present without those constants). Consider also
> asserting that the guard's comparison op isn't present in the selected LIR (e.g. no `Lir::I64GtS`
> for this case) to pin the intended behavior.

Valid point: the assertion under-specifies the property the test claims to pin. Adding a
negative-assertion on the guard's comparison op would make it actually prove elimination.
