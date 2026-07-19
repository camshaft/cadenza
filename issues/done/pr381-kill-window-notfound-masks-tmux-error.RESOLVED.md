# PR review comment — mirrored from GitHub PR #381 (Copilot inline)

- **PR:** #381 (MERGED)
- **File:** `xtask/src/fleet.rs:1270`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589595031
- **Link:** https://github.com/camshaft/cadenza/pull/381#discussion_r3589595031

## Comment (verbatim)
> kill_window() tries to distinguish "no such window" from "tmux errored", but the existence check is based on tmux_windows(), which returns an empty list on ANY tmux invocation error. That means a missing/errored tmux can still be reported as KillOutcome::NotFound, undermining the new reporting.

## Liaison triage
Follow-up to pr375's kill_window comment (3589186028): the batch added a KillOutcome::NotFound vs
error distinction, but the existence check uses tmux_windows() which returns [] on ANY tmux error, so
a missing/errored tmux is still reported as NotFound — defeating the new reporting. Fleet-tooling
territory (`xtask/src/fleet.rs` → v-fleet-tooling). Fix on `trunk`. Quote + link in queue file.
