# PR #1815 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1815 (trait-rename beat T4 — alias-bridge; mark _async shims
`#[doc(hidden)]`).

## Intra-doc link to now-`#[doc(hidden)]` `fire_due_timers_async` (Copilot, kernel.rs:520) — doc/rustdoc [VERIFIED]
> `fire_due_timers_async` is now `#[doc(hidden)]`, but this module still has rustdoc prose linking to
> `[`fire_due_timers_async`]` (e.g. `timer_armed_token_of` docs ~kernel.rs:934-940). That link now targets
> a hidden shim (confusing / potentially breaks generated docs). Repoint to `fire_due_timers`, or keep the
> shim visible (deprecated) until docs migrate.
VERIFIED on the cand branch: `fire_due_timers_async` is `#[doc(hidden)]` (kernel.rs:519), but
`timer_armed_token_of`'s doc (~:938) still has the intra-doc link `[`fire_due_timers_async`]`. A rustdoc
intra-doc link to a doc-hidden item points at a hidden target — confusing + can trip
`rustdoc::broken_intra_doc_links` depending on lint config. Repoint the link to `[`fire_due_timers`]` (the
visible target name). Same rustdoc-link pattern as #1692/#1764, here during the T4 alias-bridge window.
LOW/doc. Fix-forward.
