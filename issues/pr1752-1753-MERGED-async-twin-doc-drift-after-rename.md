# PRs #1752 + #1753 review comments — cdz-kernel/src/kernel.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1752 + #1753 (MERGED — the async-only rename: drive/fold_tip/
replay dropped their _async suffix, one path each). (The #1753 replay compile-break is filed SEPARATELY +
URGENT → pr1753-MERGED-URGENT-*.) These are the residual "async twin / sync path" doc-drift comments.

## Doc comments still say "ASYNC twin of Session::X" / "exactly as the sync path" after the sync path was removed (Copilot, kernel.rs:532/730/898 [#1752], :533/897 [#1753]) — doc
> `drive`/`fold_tip`/`replay` doc comments still describe themselves as the "ASYNC twin" of a sync method
> and reference a separate sync path, but there's now one path each (the sync twin is gone).
After the async-only collapse (one method each, no sync twin), the "async twin of Session::X" + "exactly as
the sync path" framing is stale — it implies a sibling sync method that no longer exists. Reword each to
describe the single method's behavior directly (drop "async twin" / "sync path" references). LOW/doc,
several sites across #1752 (drive:532, fold_tip:730, replay:898) + #1753 (drive:533, replay:897).
Fix-forward.
