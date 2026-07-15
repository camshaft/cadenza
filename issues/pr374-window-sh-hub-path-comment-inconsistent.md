# PR review comment — mirrored from GitHub PR #374 (Copilot inline)

- **PR:** #374 "fleet: reconcile trunk onto main + integrate the fleet-era work" (MERGED)
- **File:** `fleet/window.sh:20`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589044837
- **Link:** https://github.com/camshaft/cadenza/pull/374#discussion_r3589044837

## Comment (verbatim)
> The comment describing how HUB is computed is inconsistent with the actual path math. The script is at `<hub>/.claude/fleet/window.sh`, so `../..` resolves to `<hub>`, not `../../..`. This can mislead future edits/debugging.

## Liaison triage
Real doc/code inconsistency in fleet-tooling territory (`window.sh` is owned by `v-fleet-tooling`).
The fix lives on `trunk` even though the PR is merged. Verify the path arithmetic and correct the
comment (or the math) so they agree. Low-risk, comment-level.
