# PR #1764 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1764 (drop stale "ASYNC twin / sync path" doc — the fix for my
#1752/#1753 async-twin findings). The cleanup itself left residual stale API refs.

## Doc comments still name removed sync APIs `fire_due_timers` / `Session::deliver` → misleading + rustdoc broken-link risk (Copilot, kernel.rs:440) — doc [VERIFIED]
> kernel.rs still documents now-removed sync APIs: `fire_due_timers` in the `armed_timers` field docs and
> `Session::deliver` in the fork-for-query docs. There's no `fn fire_due_timers` or `pub fn deliver` — so
> these references are misleading and may trigger rustdoc broken-intra-doc-link warnings if denied in CI.
> Update to the async APIs (`fire_due_timers_async` / `deliver_async`) or rephrase.

VERIFIED on the cand branch: doc comments reference `fire_due_timers` (kernel.rs:62, 87, 144, 236) and
`Session::deliver` (:87, :144, :177) — but the actual methods are `fire_due_timers_async` / `deliver_async`
(:319, :315). The `[`Session::deliver`]` intra-doc link (:177) would trip `rustdoc::broken_intra_doc_links`
if denied. This async-doc-cleanup PR (fixing my #1752/#1753 findings) missed these sibling refs. Update to
the `_async` names (or rephrase to avoid naming removed symbols). LOW-MED (broken-link CI risk on the
kernel). Fix-forward.
