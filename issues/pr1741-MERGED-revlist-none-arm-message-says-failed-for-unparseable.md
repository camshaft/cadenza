# PR #1741 review comment — xtask/src/fleet.rs (v-fleet-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1741 (MERGED). #1712→#1719→#1725→#1731→#1734→#1741 rev-list chain.

## `None` arm message says "rev-list FAILED" but None now also means unparseable stdout (Copilot, fleet.rs:8125) — doc/accuracy
> The comment correctly notes `range_count == None` can come from unparseable `rev-list --count` stdout
> (via `parse().ok()`), but the later `None` arm still says "rev-list itself FAILED" and the emitted error
> always says `git rev-list … FAILED`. Misleading when stdout is unparseable but the command SUCCEEDED;
> update the wording (optionally include raw stdout).

The #1734 fix broadened the `None` meaning (spawn/exit failure OR unparseable stdout), but the None-arm's
user-facing message still says "FAILED" (implying the command failed). When rev-list succeeds but emits
unparseable output, the message misattributes it. Reword to "rev-list failed OR returned unparseable
output" and optionally include the raw stdout. LOW/doc — the running polish on the rev-list error face.
Fix-forward.
