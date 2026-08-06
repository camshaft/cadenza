# PR #2318 review — xtask/src/fleet.rs (v-fleet-tooling) — OPEN — correctness [VERIFIED-plausible, MED]

https://github.com/camshaft/cadenza/pull/2318 (watchdog warns loudly when its own xtask source lags trunk —
stale-binary guard; branch cand/v-fleet-tooling-244f4dd17). Copilot 1 inline (id 3724941272, fleet.rs:2945).

## the self-staleness check runs `git rev-list --count HEAD..trunk` against `fleet.repo` (the hub/common git dir), whose `HEAD` is the HUB's, not the current linked worktree's → in the bare-hub + linked-worktree setup it often reports 0 and fails to warn even when the watchdog binary was built from a stale worktree (Copilot, fleet.rs:2945) — correctness [VERIFIED-plausible, MED]
> The self-staleness check compares `HEAD..trunk` using `fleet.repo` (the hub/common git dir). In a bare-hub
> + linked-worktrees setup, `git -C <hub>` uses the hub's own `HEAD`, not the current worktree's `HEAD`, so
> this will often report `0` and fail to warn even when the watchdog binary was built from a stale worktree.
> Run the rev-list in the current worktree root instead (parent of `fleet.src`).

VERIFIED the mechanism in the #2318 diff: `xtask_commits_behind_trunk(&fleet.repo)` (diff:131) runs
`git … rev-list --count HEAD..trunk -- xtask` (diff:151) with the git dir = `fleet.repo`. In this fleet's
bare-hub + linked-worktree topology, `fleet.repo` is the hub/common git dir, so `HEAD` there is the hub's
HEAD — NOT the worktree HEAD the watchdog binary was actually built from. So the count measures the wrong
ref: a watchdog running from a stale WORKTREE (its `HEAD` behind trunk) would still see the hub's `HEAD` at
trunk → report 0 → NO warning, defeating the guard's purpose. MED / correctness (the stale-binary guard
silently no-ops in exactly the topology it's meant to protect).

RELAYED AS PLAUSIBLE: v-fleet-tooling owns the bare-hub mechanics and knows precisely how `fleet.repo` vs
`fleet.src` resolve at watchdog runtime — this is squarely their call to confirm the HEAD-source. Fix per
Copilot: run the rev-list in the current WORKTREE root (parent of `fleet.src`) so `HEAD` is the worktree's
actual built-from commit. Worth confirming against how the watchdog is invoked (cwd / which worktree).
v-fleet-tooling owns xtask/src/fleet.rs. PR OPEN → foldable pre-merge. (Ties the watchdog self-heal arc; a
guard that can't see its own staleness is worse than none — worth getting the HEAD-source right.)
