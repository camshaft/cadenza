# PRs #1757 + #1751 review comments — .github/workflows/*.yml (v-fleet-tooling / v-agent-harness-host) — mixed

## PR #1757 (checks.yml:250, v-fleet-tooling) — CI comment says "floats rustc/wasm-tools" but workflow pins the toolchain — doc
Reword the stale "floats rustc/wasm-tools → different versions" comment to match the now-pinned toolchain.
LOW/doc.

## PR #1751 (checks.yml:315, v-agent-harness-host) — comment says guest deps "aren't lock-pinned" but step now builds with --locked + committed Cargo.lock — doc
Same stale-comment class as the #1747 checks.yml:258 nit. Update to reflect the lock-pinned build. LOW/doc.

## PR #1751 (issues/pr1731-*, issues/pr1725-* :1) — github-liaison's OWN queue-file headers said OPEN while filename said ADDRESSED — FIXED BY LIAISON
Copilot flagged (via the issues/ archive mirror) that my two ADDRESSED-named queue files still had "— OPEN"
headers. VERIFIED + FIXED directly (both headers → ADDRESSED with the delta/chain-closed note). No owner
action — these are my artifacts. Noted so the thread reads resolved.
