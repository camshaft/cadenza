# PRs #1710 + #1714 review comments — cdz-kernel/src/kernel.rs tests (v-agent-harness) — MERGED, fix-forward

## PR #1714: genesis-hash label typo `guery-then-seed-v1` (Copilot, kernel.rs:2268) — typo
https://github.com/camshaft/cadenza/pull/1714 — `guery` → `query` in the test's genesis hash label (a
greppable string; fix for readability). LOWEST.

## PR #1710: extension/register-by-string collision test uses a control-plane family (Copilot, kernel.rs:2355) — test-precision
https://github.com/camshaft/cadenza/pull/1710 — the test describes an EXTENSION / register-by-string
family colliding with a real `emit`, but uses the control-plane `control/capabilities` family. A
control/* family is partitioned out BEFORE the emit-collision path (per the #1599/#1614 control-vs-effect
split), so it may not exercise the extension-vs-emit collision it intends. Use a genuine (non-control)
extension family string so the test hits the register-by-string routing it describes. LOW/test-precision —
recommend v-agent-harness confirm the family choice actually reaches the collision path.
