# PR review comment — mirrored from GitHub PR #426 (Copilot inline)

- **PR:** #426 "fleet: fiftieth batch (coverage-floor max-merge driver, …)" (MERGED)
- **File:** `xtask/src/fleet.rs:610` (`register_merge_drivers`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592095864
- **Link:** https://github.com/camshaft/cadenza/pull/426#discussion_r3592095864

## Comment (verbatim)
> `register_merge_drivers` currently uses `std::env::current_exe()` to build the merge-driver command. Because all worktrees share the hub's `.git/config` but *don't* necessarily share the same `target/` (and the merge may run from a different worktree than the one that ran `fleet up`), this can register a driver pointing at a non-existent binary path, making the driver fail when it's needed. Also, `.status().ok()` silently ignores both spawn failures and non-zero exit codes, so a broken registration is hard to notice.

## Liaison triage — CONFIRMED plausible against trunk
This is the merge-driver that implements the `merge=union`/auto-dedup for the gate baselines (the
durable fix from my earlier gate-baseline dup thread). Concern: `register_merge_drivers` builds the
driver command from `std::env::current_exe()`, but worktrees share the hub `.git/config` while NOT
necessarily sharing `target/` — so a merge running from a different worktree than the one that ran
`fleet up` can invoke a driver path that doesn't exist → the driver silently fails and the baselines
merge WITHOUT the dedup/max-merge (reintroducing the duplicate-description problem). `.status().ok()`
swallowing spawn/exit failures makes a broken registration invisible. Fleet-tooling (v-fleet-tooling).
FIX: register a stable/resolvable driver path (or a `cargo run`/`xtask`-relative invocation that works
from any worktree), and surface a registration/driver failure instead of `.ok()`-swallowing it. Fix on
`trunk`. Quote + link in queue file.
