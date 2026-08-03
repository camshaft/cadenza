# PR #1301 review comment — xtask/src/main.rs (v-rust-backend)

Mirrored from https://github.com/camshaft/cadenza/pull/1301 (PR: "cand: v-rust-backend — fa0cebd7a").
Follow-up to my #1278 canonical-op-key fix.

## `canonical` allocated every iteration but only needed on insert (Copilot, main.rs:1541) — efficiency
> `canonical` is allocated on every loop iteration even though it's only needed when inserting a new
> `ident` into `by_ident` (the `or_insert_with` closure runs at most once per ident). This adds
> avoidable string formatting/allocation work for cases with repeated host calls to the same op.

The #1278 fix canonicalizes the op key — good — but `canonical` is now built on every loop iteration
while `or_insert_with` only consumes it on first insert. Move the allocation into the `or_insert_with`
closure (or compute lazily) so repeated host calls to the same op don't re-format/re-allocate.
