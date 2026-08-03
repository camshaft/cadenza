# Design: CI-gated parallel-lane integration for pr-sync

Status: DRAFT (v-fleet-tooling, 2026-08-02). Owner: `v-fleet-tooling`. Priority #1.
Supersedes the single-thread local-gate land model in `fleet/loops/pr-sync.md`.

## Why (operator directives, 2026-08-02, relayed by concierge)

Three escalating directives, same thrust — the single-thread local-gate integrator is the
throughput ceiling and must go:

1. "pr sync just rely on the GitHub ci instead of running it all locally. It's going to run in
   GitHub regardless but the nice thing there is it's all parallel. The current setup is just kinda
   redundant."
2. "I would rather just have it push and rely ENTIRELY on the GitHub actions for the gate. The
   approach we have now is just so slow." + "the pr sync would just query the GitHub ci status to
   know what to do with it."
3. "We can have multiple PRs going in parallel categorized by the likelihood of causing issues. So
   documentation changes can go in one. Or corpus changes/fixes in another set. Basically it's up to
   the pr sync agent to try to make sure things are flowing. But this single thread approach just
   isn't working anymore."

Net: pr-sync stops running a local gate and becomes a **flow manager over parallel, CI-gated PRs**,
categorized into **lanes** by collision-risk so low-risk work (docs, corpus) lands freely in
parallel while higher-risk work (compiler/runtime) is ordered within its lane.

## The two facts this design stands on (verified)

- **CI already runs the full gate in parallel.** `.github/workflows/ci.yml` triggers
  `checks.yml` on `pull_request → main`, fanning out the real workflow jobs — `rustfmt`, `clippy`,
  `test` (ubuntu + macos), `codegen`, `wasm-runtime`, **`gate`** (the corpus gate), `cad-tests`,
  `cdz-kernel`, `bench` (the allocation bench), `guide-examples` — as separate jobs. This is exactly
  what `cargo xtask check` runs *serially* locally — so the local gate is genuinely redundant. (Job
  ids are the real `checks.yml` names so this doesn't drift — PR #1083 review.)
- **Peers `fleet sync` reset onto the LOCAL `trunk`** bare-hub ref (verified in `fleet.rs`). So a
  broken local trunk breaks every peer's base *immediately*, before CI ever sees it. This is the
  reason the local gate existed. The CI-gated model resolves it a different way: **`trunk` is
  advanced ONLY after that candidate's CI is green**, so a broken tree never lands on local trunk at
  all — no local gate needed to protect it. Forward-only + trunk-never-broken are PRESERVED per land.

## The model

Refined against pr-sync's pilot (it received the same operator directive directly, 2026-08-02, and
manually validated the pipeline while draining the ~30-MR backlog). The automated executor
(`cargo xtask fleet schedule-pass --execute`) then replaced the manual loop (cutover 2026-08-03).
Key mechanic: **GitHub auto-merges each candidate on CI-green** — pr-sync does NOT merge on a green
poll; it enables auto-merge at push time and only polls to detect the merge, then advances local
`trunk` by cherry-picking that PR's own squash `mergeCommit.oid`.

**⚠ Trunk advance is a CHERRY-PICK of the PR's `mergeCommit.oid`, NOT a fast-forward from
`origin/main` (learned in the cutover, 2026-08-03).** `trunk` and `origin/main` are permanently
TREE-EQUAL but COMMIT-DISTINCT (every candidate is re-parented onto `origin/main` before its PR), so
a literal FF is impossible and `trunk..origin/main` is `origin/main`'s WHOLE divergent history, not
this PR. The one correct advance is `git cherry-pick <mergeCommit.oid>` (from
`gh pr view <n> --json mergeCommit --jq .mergeCommit.oid`) onto `trunk` — its parent's tree == trunk's
tree, so it applies cleanly and advances trunk by exactly this PR; multi-merge windows are handled by
reaping each PR separately. The cherry-pick MUST run in the worktree that has `trunk` CHECKED OUT
(pr-sync's), never the bare hub (no work tree → git errors out, misreported as a false conflict). See the
smoke-test bug log below.

```
MR arrives → categorize into a LANE → re-parent sender --ref onto origin/main in a scratch worktree
          → push cand/<agent>-<sha>, `gh pr create --base main`, `gh pr merge --squash --auto --delete-branch`
          → GitHub Actions gates it IN PARALLEL with every other in-flight candidate
          → GitHub AUTO-MERGES the PR to origin/main the instant its CI is green (no pr-sync action)
          → pr-sync POLLS (fleet ci-status): PR merged  → cherry-pick its mergeCommit.oid onto `trunk` (in the
                                                          trunk worktree) + ack `merged` + retire the record
                                             CI red (merge-REQUIRED job) → ack `reject` (failing job + run URL)
          → NO local gate, NO combined-tree bisect (a red candidate fails its OWN PR alone, blocks nothing)
```

pr-sync's tick becomes a **scheduler pass**, not a serial gate loop: reap concluded PRs
(merged→cherry-pick-mergeCommit.oid-onto-trunk+ack / required-red→reject), then top up in-flight
capacity by pushing new candidates from the queue, respecting per-lane ordering + a global in-flight
cap. After the reap it runs `git remote prune origin` (drops stale cand remote-tracking refs whose branch
auto-deleted on merge). Dropping the bisect is a direct
win: today one red MR in a batch forces ~log₂(N) re-gates to isolate; in the new model it just fails
its own PR in isolation and every other candidate proceeds untouched.

### In-flight state (map MR ↔ candidate PR)

pr-sync needs a durable STATE file (`.claude/fleet/ci-dispatch/<mr-file>.json` →
`{cand_branch, pr_number, agent, ref, lane, pushed_at}`) so a scheduler pass can: (a) not
double-dispatch an MR already in flight, (b) map a merged/red PR back to its MR to `ack`, (c) resume
across ticks/restarts (the manifest, not memory, is the truth — same principle as the registry).

### Lanes (categorize by collision-risk on changed paths) — DATA-DRIVEN, extensible

Operator ruling (2026-08-02): "It doesn't have to be just those two lanes … feel free to scale it up
as much as needed. Those were just examples." So the taxonomy is NOT a fixed enum — it's an ORDERED
DATA TABLE (`LANE_RULES` in `fleet.rs`: matcher → lane name + parallel flag), and **adding a lane is
adding one row**. More disjoint-territory lanes ⟹ more parallelism, which is the operator's intent.

A candidate's lane is a pure function of its changed-path set (`git show --name-only --format= <ref>`,
folded by `lane_of`). First-match-wins, ordered most-specific/lowest-collision first. Current table:

| Lane            | Path territory (first match wins, top-down)          | Lands       |
|-----------------|------------------------------------------------------|-------------|
| `baseline`      | EXACT: the 3 `spec/semantics/.gate-baseline*` files  | serialize — ~6 agents share these files |
| `rcdzc-tests`   | EXACT: `implementation/seed/crates/rcdzc/src/tests.rs` | serialize — ~6 agents append to this file |
| `corpus`        | `spec/**`, `*.sexp` (NON-baseline)                   | PARALLEL — independent cases |
| `cad`           | `implementation/cad/**`                              | PARALLEL — disjoint leaf |
| `music`         | `implementation/music/**`                            | PARALLEL — disjoint leaf |
| `des`           | `implementation/des/**`                              | PARALLEL — disjoint leaf |
| `choreography`  | `implementation/choreography/**`                     | PARALLEL — disjoint leaf |
| `iterators`     | `implementation/iterators/**`                        | PARALLEL — disjoint leaf |
| `compiler-ml`   | `implementation/compiler-ml/**`                      | PARALLEL — disjoint leaf |
| `fleet-tooling` | `xtask/**`                                           | serialize — shared `fleet.rs` |
| `docs`          | `guide/**`, `playground/**`, `skills/**`, `templates/**`, `design/**`, `**/*.md` | PARALLEL |
| `code`          | rest of `implementation/**` (seed compiler, crates, runtime, wit) | serialize — shared core |
| `fleet-tooling` | rest of `fleet/**` (window.sh, roster, slack-bridge) | serialize |
| `mixed`         | spans >1 lane, or unrecognized territory             | serialize globally (safest) |

Each LEAF subsystem (`cad`/`music`/`des`/…) owns a disjoint `implementation/<x>/` tree, so their
candidates can never collide with each other → all parallel. The shared compiler/runtime CORE and the
fleet tooling serialize (many candidates touch the same files). Add a new subsystem = add a row.

**The real collision unit is a set of shared HOT FILES, not a directory** (pr-sync pilot evidence).
A rule can match EXACT files (`exacts`), checked before prefixes/suffixes. Two such lanes exist
because ~6 agents each contend on one shared file: `baseline` (the 3 `.gate-baseline*` files) and
`rcdzc-tests` (`rcdzc/src/tests.rs`). They are SERIALIZED but SEPARATE from `corpus`/`code`, so a
`.sexp`-only change (not a baseline) or a non-`tests.rs` code change still lands in PARALLEL with an
in-flight baseline/tests.rs MR. Folding them into corpus/code would FALSE-serialize disjoint work and
lose throughput — the finer the *accurate* lane cut, the more parallelism.

Rationale: the lane fold is a **pure function of the changed path set** → unit-testable, no false
"low-risk". A ref spanning two lanes (even two PARALLEL leaves, e.g. `cad`+`music`) is `mixed`
(serialized globally) — it touches multiple territories and could collide with either.

**Combinable lane sets** (avoid false `mixed`): some multi-lane spans have a natural CONTAINING lane
and shouldn't fall to global `mixed`. Today's rule: `{corpus, baseline}` → `baseline`. That's the
NORMAL corpus workflow — editing a `.sexp` case updates the shared `.gate-baseline*` files in the same
commit. It collides on the baseline hot files (→ serialize as `baseline`), but folding it to `mixed`
would needlessly serialize it against UNRELATED mixed work (a docs+code MR). Measured on the live
queue: this drops `mixed` from 11/29 → 5/29 (the other 6 become `baseline`), a real parallelism win.
Any *other* multi-lane span (truly unrelated territories) stays `mixed`.

### Cross-lane ordering — the hard part (the operator's "make sure things are flowing")

Two candidate PRs can both be CI-green against `origin/main` yet conflict once one lands (the second
was gated against a trunk that no longer exists). Handling, cheapest first:

1. **Independent lanes never block each other.** `docs`/`corpus` candidates touch disjoint path sets
   from each other and from `code`, so their green PRs land immediately regardless of order.
2. **Within a serialized lane**, at most ONE candidate is in flight at a time (the next is pushed
   only after the current lands/rejects) — no intra-lane conflict possible.
3. **Cross-lane collision on shared files** (e.g. a `mixed` PR and a `code` PR both touch a runtime
   file): when a green PR is about to land, re-check its `mergeCommit.oid` still cherry-picks CLEANLY
   onto the CURRENT trunk. If trunk advanced under it since CI started AND the pick now conflicts →
   **re-base the candidate on the new trunk, re-push, re-gate** (its CI re-runs), rather than force a
   possibly-broken merge. This is the "re-gate if another landed first" the operator described. (In
   the executor, a conflicting cherry-pick aborts + leaves the record in-flight for a retry — the
   fail-safe never corrupts trunk.)
4. A candidate whose lane predicate says it's disjoint from everything currently in flight skips the
   re-check (provably can't collide).

Landing is a **clean cherry-pick of the PR's `mergeCommit.oid` onto `trunk` only** — never a conflict
resolution by pr-sync (that stays the author's job, same as today's reject-on-conflict; a conflicting
pick aborts and the MR is left for the author to rebase).

**Reap edge case — a CLOSED-but-not-merged PR.** ✅ DECISION-LAYER DONE (landed additively ahead of the
I4 executor). A candidate PR can be **CLOSED without merging and without a red CI** (manually closed,
superseded, or its branch deleted) — under the old two-input `reap_action(merged, verdict)` that read
`merged=false` + a pending/green verdict → `KeepWaiting` FOREVER, a stuck in-flight slot. FIXED: reap
now takes a `PrState {Merged, Closed, Open}` (via `parse_pr_state` on `gh pr view --json state`, pure +
unit-tested) instead of a bare `merged` bool, and `reap_action(state, verdict)` resolves
`Closed → Reject` regardless of the stale CI verdict (ack the MR so its sender can resend + free the
slot). `pr_state_and_verdict` fails safe to `(Open, NoChecks)` on a gh error — an error manufactures
neither a land nor a reject. The read-only `schedule-plan` preview already surfaces this (distinct
"PR closed unmerged → free slot" REJECT reason). The I4 executor consumes the same primitive when it lands.
  - **Closed-WHILE-polling race** (PR #1160 review): beyond a PR closed between passes, the poll loop
    must also handle a PR closed/merged DURING a single poll cycle — re-fetch `state` on each poll rather
    than caching it, and treat a mid-poll `CLOSED`/`MERGED` as terminal so the poller never waits forever
    on a PR that vanished under it. I4 requirement: each reap poll reads fresh `(state, checks)`; no
    per-candidate poll caches the pre-poll state.

## Pilot learnings (pr-sync's manual validation, 2026-08-02 — bake these into I3/I4)

pr-sync manually validated the pipeline end-to-end (4 candidate PRs #1037–1040 in flight, all
auto-merge-armed, gating in parallel; my base landed via #1038). Hard-won gotchas:

1. **Scratch worktree is DETACHED HEAD → push needs the FULL refspec**
   `HEAD:refs/heads/cand/<agent>-<ref>`. A bare `HEAD:cand/…` errors ("src is a commit object, did
   you mean refs/heads/…"). `publish-candidate` must use the full form.
2. **Cherry-pick the SINGLE `--ref` commit** (not the branch range) onto a fresh
   `git worktree add --detach <scratch> origin/main`. Clean 1-file picks in practice.
3. **`gh pr create` prints only the URL** — scrape the PR number with `grep -oE '[0-9]+$'`.
4. **Real collision confirmed:** two candidates both touching `implementation/seed/crates/cdz/src/main.rs`
   (v-choreography `0c7f941ed` + v-cdz-tooling `4feac9a35`) — `lane_of` classifies BOTH as `code`
   (serialized), so the same-lane serialize rule (≤1 in flight) correctly holds the 2nd until the 1st
   merges. VERIFIED live with `fleet lane-of`. (The harder cross-lane-shared-file case is the
   FF-recheck in ordering rule 3 above.)
5. **In-flight cap = 8–10** (operator ruling: "fine with a max of 8-10 PRs in parallel, especially if
   there's no collision risk"). I4 default 8–10, flag-tunable; dynamic: up to the cap when lanes are
   disjoint, throttle when candidates contend. pr-sync piloted at the floor (4) pending the ruling.
   The cap is a **DISPATCH THROTTLE, never a cancel** (operator 2026-08-02): it only bounds how many
   NEW candidates launch — it NEVER cancels an already-dispatched candidate to get back under the cap,
   because GitHub Actions runs a cancelled PR's CI jobs anyway (cancelling wastes the in-flight work +
   delays that MR for zero runner savings). In-flight candidates always run to completion (green→land /
   red→reject). `schedule_dispatch` encodes this: `slots = cap.saturating_sub(current_in_flight)` → at
   or over the cap, 0 new dispatches, and it only ever returns NEW picks (never touches in-flight).
6. **`gh` account = `camshaft`; no branch protection / no native merge-queue on `main`; auto-merge
   works.** So `gh pr merge --squash --auto` is the merge mechanism (no merge-queue to integrate with).
7. **State convention adopted:** `.claude/fleet/ci-dispatch/<mr-file>.json` (pr-sync is already
   writing these) — the MR↔candidate-PR map I3 writes and I4's reaper reads.
8. **BOTTLENECK = runner concurrency (jobs-in-flight), NOT PRs-in-flight, NOT arch, NOT collisions**
   (pilot at cap-8: 6 landed / 1 CI-red-rejected / 1 dup, 8 continuously in flight, backlog fed as
   fast as drained = healthy steady state). On the freshest PR ~14/16 jobs were QUEUED (incl. ubuntu
   clippy/rustfmt), so it's global GitHub Actions runner saturation: 8 PRs × ~16 jobs ≈ 128 jobs
   behind the account runner limit. IMPLICATIONS for I4: (a) raising the cap ABOVE 8 likely won't
   help — more PRs just deepen the shared job queue and may trigger GitHub throttling/cancellation;
   8 is the sweet spot the operator already picked. (b) The real tuning knob is JOBS-in-flight, not
   PRs — so the highest-leverage optimization is a **per-lane lighter check subset**: a low-risk lane
   (docs/corpus) need not run the full ~16-job arm gate — a docs PR gating on `guide-examples` + fmt
   + the relevant subset frees runners for `code` PRs that need the full gate. This composes with the
   lane model (each lane declares its required check set). Raised to the operator as a follow-on
   optimization; NOT urgent (pipeline is healthy at cap-8). Size I3/I4 for a SUSTAINED ~30-deep queue,
   not a one-time drain.
9. **pr-sync's OWN housekeeping commits need a publish path** (pr-sync note, on record). The
   `fleet archive` step (charter step 4: mirror `.claude/fleet/queue/` → tracked `issues/`) makes a
   trunk-only commit that is NOT an agent MR, so it has no candidate-PR route to `origin/main`. In the
   old serial model it rode out with each publish cycle; in the CI-gated model it would waste a full
   16-job CI slot + scarce arm runners for zero correctness value, so pr-sync currently DEFERS it
   (verified: `git diff trunk origin/main -- . :(exclude)issues/` is empty every tick — the only
   trunk-only content is the `issues/` reproducer archive, which is safe on trunk; peers sync fine).
   The durable model needs SOME path so reproducers reach `origin/main` eventually. Options: (a) a
   dedicated `fleet publish-housekeeping` that pushes `issues/` straight to `origin/main` with MINIMAL
   checks (a `housekeeping`/`archive` lane = the per-lane-check-subset idea applied — this is pr-sync's
   and my lean); (b) piggyback the archive onto the next agent candidate PR already gating; (c) a
   periodic low-priority direct push during a lull. NOT urgent (issues/-only, non-blocking); fold into
   I5 hardening.

## Increments (each independently landable + gated)

- **I1 — CI-verdict primitive.** ✅ LANDED-LOCALLY (`3a6948994`, held): `fleet ci-status
  <pr|branch>` + pure `ci_verdict_from_buckets` fold (`CiVerdict{Green,Red,Pending,NoChecks}`,
  empty→NoChecks-never-Green). The poll primitive everything else calls.
- **I2 — lane categorizer.** Pure `lane_of(paths: &[String]) -> Lane` + `fleet lane-of <ref>` to
  inspect. Unit-tested against the table above. No land-path change.
- **I3 — publish-candidate.** `fleet publish-candidate <ref>`: re-parent the ref's tree on
  `origin/main` in a scratch worktree (reuse the existing publish plumbing in `pr-sync.md` step 3),
  push `cand/<agent>-<sha>`, `gh pr create --base main`, `gh pr merge --squash --auto --delete-branch`
  (GitHub merges on green). Records the in-flight state entry (MR↔PR). Returns the PR handle. No
  trunk move.
- **I4 — the scheduler executor.** ✅ LANDED + CUT OVER (2026-08-03). Replaced pr-sync's local
  `gate_subset` land loop with `schedule_pass_execute`: reap-concluded (PR merged → cherry-pick its
  `mergeCommit.oid` onto `trunk` in the trunk worktree + `ack merged` + retire the record; merge-REQUIRED
  CI red or PR CLOSED-unmerged → `ack reject` with failing job + run URL; non-required red → keep
  waiting) → `git remote prune origin` → top-up in-flight per lane under a global cap. `pr-sync.md` +
  `AGENTS-fleet.md` updated. (No bisect — a red fails its own PR alone.) Hardened through 4 `--execute`
  smoke rounds — see the bug log below.
- **I5 — hardening.** ✅ Landed the reap-correctness set (own mergeCommit.oid #1532, fetch-before-pick
  #1549, trunk-worktree-not-bare-hub #1591, dispatch mis-count #1597, preview/executor guard-mirror
  #1600, stale-cand-ref auto-prune #1609) + record auto-retire. Remaining/optional: in-flight cap
  tuning, CI timeout → reject-with-timeout, removing the now-dead local-gate code (`gate_subset`,
  `gate-batch` bisect) once fully proven.

Each increment lands as its own merge-request (per-commit cadence). Trunk-safety invariant holds at
every step: nothing advances trunk except a CI-green candidate, and a reap that cannot cleanly advance
(cherry-pick conflict / gh error) leaves the record in-flight rather than corrupting trunk.

## Executor smoke-test bug log (the cutover, 2026-08-03)

The executor was hardened behind pr-sync's LIVE `schedule-pass --execute` smoke-tests — each bug was
an ENVIRONMENT bug invisible in a full-stack worktree (unit tests verified the LOGIC; only the
consumer peer's real environment surfaced the WIRING). Fix-forward one-bug-per-round, zero trunk
corruption:
- **BUG-1 / 1a / 1b** — advance-trunk model: a literal FF from `origin/main` is impossible (re-parent
  model → tree-equal-but-commit-distinct); must cherry-pick the PR's OWN `mergeCommit.oid` (#1532, not
  `origin/main`'s tip or the range), after `git fetch origin <oid>` so its sequential-squash parent is
  local (#1549).
- **BUG-2** — `gh pr create --fill` fails (no local `origin/main..head` range on a fresh cand branch);
  use explicit `--title`/`--body`.
- **BUG-3** — the cherry-pick ran in the BARE hub (`fleet.repo`, no work tree) → git errors out "must be
  run in a work tree", misreported as a false conflict; run the mutating ops in the worktree that has
  `trunk` checked out (#1591, via `parse_trunk_worktree` over `git worktree list --porcelain`).
- **Guard + count nits** — the re-dispatch idempotency guard matched agent-alone across all records
  (blocked a new different-ref MR from the same agent); scope to `dispatch_is_in_flight` + `refs_match`
  on the ref (#1591). The dispatch counter incremented even when the guard bailed; `publish_candidate`
  now returns bool, counted only on a real dispatch (#1597). `dispatch_plan` (the preview) must mirror
  the executor guard exactly or the two diverge (#1600).
- **Housekeeping** — reaped ci-dispatch records are DELETED after ack (not left resolved-in-place); the
  reap `git remote prune origin`s stale cand remote-tracking refs after a merge (#1609).

## Open questions (need an operator ruling — raised via `ask`)

1. **In-flight cap.** How many concurrent candidate PRs / CI runs are acceptable (GitHub Actions
   concurrency + cost)? Propose a small default (e.g. 4) tunable by a flag.
2. **Lane taxonomy.** Are the 5 lanes above the right cut, or does the operator want a specific set
   (they named "documentation" and "corpus changes/fixes" explicitly — those are covered)?
3. **CI latency vs. today.** The bet is parallel CI beats the ~10-15min serial drain even with the
   push→poll→merge round-trip. Accept the per-PR CI latency (minutes) as the cost of unbounded
   parallelism? (I believe yes per directive #2's "so slow" framing, but confirming.)
4. **Publish/origin-main relationship.** RESOLVED via pr-sync's auto-merge mechanic: each candidate
   PR auto-merges to `origin/main` on CI-green, and pr-sync then FFs local `trunk` from
   `origin/main`. So `origin/main` advances first (per green candidate), and local `trunk` follows as
   the peer-sync base — KEEPING `trunk` as what peers `fleet sync` onto, least disruption. (Confirming
   with the operator, but this is the natural shape and matches the existing publish direction.)
