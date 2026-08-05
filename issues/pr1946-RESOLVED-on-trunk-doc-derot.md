# PR #1946 review — xtask/src/fleet.rs (v-fleet-tooling) — LOW cosmetic

https://github.com/camshaft/cadenza/pull/1946 (capture confirmed_idle FRESH just before restart — the
#1941 fix I flagged, taking the shrink-the-window option). Copilot (id 3709406045) flags a doc-nit on the
new comment.

## comment hard-codes the `:3016` line ref + a "sub-ms" TOCTOU claim, both fragile (Copilot, fleet.rs:3299) — clarity/cosmetic [VERIFIED, LOW]
> This comment anchors the "sweep-top `pane` snapshot" to a specific line number (`:3016`) and claims the
> remaining TOCTOU window is "sub-ms". Both are likely to become inaccurate as the file changes and as
> `tmux capture-pane` latency varies. Consider rewording to reference the earlier snapshot structurally
> (not by line) and avoid a hard latency claim.

Fair LOW cosmetic on the comment v-fleet-tooling just wrote for the fix. A line-number reference rots as
fleet.rs changes; "sub-ms" is an unverifiable latency assertion (tmux capture latency varies by load). No
behavioral impact — the code is correct. Reword to reference the earlier capture structurally (e.g. "the
sweep-top pane snapshot used for the drain-stall/saturation signals") and soften the window claim (e.g.
"narrow capture→restart TOCTOU" without a hard number). v-fleet-tooling's call on their own comment
wording — relaying, not pressing.
