# PR review comment — mirrored from GitHub PR #393 (Copilot inline) — SECURITY-HYGIENE

- **PR:** #393 (MERGED)
- **File:** `fleet/slack-bridge/src/config.rs` (`SlackTokens` @19, `Config` @28)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3590413256, 3590413277
- **Links:** https://github.com/camshaft/cadenza/pull/393#discussion_r3590413256 , #discussion_r3590413277

## Comments (verbatim)
> `SlackTokens` derives `Debug`, which risks accidentally leaking Slack credentials into logs (e.g., via `{:?}` in an error path). Since these are secrets, it's safer to avoid `Debug` entirely (or implement a redacting `Debug`).
>
> `Config` derives `Debug` while containing `bot_token`/`app_token`. Even if you don't currently log it, deriving `Debug` makes accidental secret disclosure much easier. Prefer removing `Debug` (or redacting it).

## Liaison triage — CONFIRMED against trunk
Confirmed: both `SlackTokens` (bot_token/app_token: String) and `Config` (bot_token/app_token:
Option<String>) carry `#[derive(Debug, Clone, PartialEq, Eq)]`. A stray `{:?}` (e.g. an error path,
a `dbg!`, a panic that formats state) would print the raw `xoxb-…`/`xapp-…` secrets into logs. Secret-
handling hygiene in the slack-bridge (v-fleet-tooling territory). FIX: drop the `Debug` derive on both
(or implement a redacting `Debug` that prints `SlackTokens { bot_token: "***", … }`). Fix on `trunk`.
Quotes + links in queue file.
