# PR review comment — mirrored from GitHub PR #375 (Copilot inline)

- **PR:** #375 (MERGED)
- **File:** `xtask/src/fleet.rs:672`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589186028
- **Link:** https://github.com/camshaft/cadenza/pull/375#discussion_r3589186028

## Comment (verbatim)
> When `kill_window(...)` returns `false`, it can mean either "window not found" or "tmux errored" (the helper intentionally conflates these). The current user-facing message only mentions the window already being closed or not being in tmux, which can mislead debugging when `tmux` is missing or the session lookup fails.

## Liaison triage
Fleet-tooling diagnostic-quality point (`xtask/src/fleet.rs` → `v-fleet-tooling`). Real but minor:
the conflated `false` return produces a message that misleads when tmux is genuinely absent/errored.
Worth disambiguating the message. Low-risk.
