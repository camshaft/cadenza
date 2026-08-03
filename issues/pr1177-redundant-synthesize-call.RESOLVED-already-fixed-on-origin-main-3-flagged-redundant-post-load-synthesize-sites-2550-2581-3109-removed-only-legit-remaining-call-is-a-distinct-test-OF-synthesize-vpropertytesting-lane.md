# PR #1177 review comment — rcdzc/src/proptest_gen.rs (v-property-testing)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1177
(PR: "cand: v-property-testing — manifest+proptest_gen").

## Redundant explicit `synthesize` call in tests (Copilot, proptest_gen.rs:2550, also :2581, :3109) — test-simplification
> `Db::load` already calls `crate::proptest_gen::synthesize` during load (db.rs:2077). Calling
> `super::synthesize` here is redundant and makes the test depend on `synthesize` remaining fully
> idempotent; you can simplify by loading directly from parse.

At three sites the tests call `super::synthesize` explicitly after `Db::load` already ran it — the
extra call is a no-op only if `synthesize` stays perfectly idempotent, which couples the tests to
that property unnecessarily. Simplify to load directly from parse and drop the redundant call.
