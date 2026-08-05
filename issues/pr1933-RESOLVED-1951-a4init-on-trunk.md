# PR #1933 review — rcdzc/src/effects.rs (v-effects) — MERGED — miscompile-class [VERIFIED-PLAUSIBLE, LIVE ON TRUNK]

https://github.com/camshaft/cadenza/pull/1933 — MERGED 2026-08-04T03:33:12Z (adv-69 a4 sub-face: a
block-wrapped outer-effect perform in a let-init inside a NESTED handle body now declines). Copilot (id
3709224081) flags that the SAME fix descends into the nested handle's `body` but skips its `init` — the
next sub-face of the same miscompile class. LIVE on trunk (effects.rs:3502).

## nested-`Handle` scan descends into `body` but SKIPS `init`; `init` runs under the outer handler too → a block-wrapped outer-op perform in the inner handle's `init` can still drop the outer advance (Copilot, effects.rs:3502) — miscompile-class [VERIFIED-PLAUSIBLE]
> When encountering a nested `Handle`, the scan now descends into the inner handle's `body` but skips its
> `init`. `init` is evaluated as part of the `handle` expression (see `eval.rs`'s `Resolved::Handle {
> init, arms, body }` traversal), so a block-wrapped outer-effect perform inside the inner handle's `init`
> can still be missed by the outer scan, potentially reintroducing the same class of silent state-drop
> miscompile in a slightly different position.

VERIFIED against trunk. `body_has_block_wrapped_let_init_branch_perform` (effects.rs:3487) does:
`if let Resolved::Handle { body, .. } = resolved_of(db, node) { return …(db, body, ctx); }` — it binds
ONLY `body` and early-returns, so the inner handle's `init` sub-tree is never scanned under the outer
`ctx`. The fix's own comment argues the inner BODY "still runs UNDER this (outer) handler" and so must be
scanned; by the same dynamic-extent reasoning the inner handle's `init` is ALSO evaluated under the outer
handler (it's the value being handled by the inner handler, evaluated before/at the inner discharge, in
the outer's extent). Confirmed the eval order in eval.rs: `Resolved::Handle { init, arms, body }`
(eval.rs:1705) passes `init` to `reduce_handle(g, init, &arms, body)` — init is part of the handle
expression's evaluation, not deferred outside the outer handler. So a `let`-init block-wrapped perform of
the OUTER discharged op sitting in the inner handle's `init` matches the exact a4 shape but escapes the
scan → the same silent outer-advance-drop miscompile in a slightly different position.

The ctx-keyed safety argument the PR relies on carries over cleanly: `block_wrapped_branch_performs` is
keyed on the OUTER op, so also descending into the inner `init` (keeping the outer `ctx`) fires only on an
outer-op perform and cannot over-decline the inner handler's own shapes. Fix: in the `Resolved::Handle`
arm, scan BOTH `init` and `body` (`…(db, init, ctx) || …(db, body, ctx)`), not `body` alone. MED —
verified-plausible miscompile-class; v-effects should confirm with a witness (block-wrapped outer-op
perform in an inner-handle let-init) and either add the decline + a corpus pin or explain why init can't
carry the shape. v-effects owns effects.rs (authored the #1933 a4 face).
