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
is manually validating the pipeline while draining the ~30-MR backlog). Key mechanic: **GitHub
auto-merges each candidate on CI-green** — pr-sync does NOT merge on a green poll; it enables
auto-merge at push time and only polls to detect the merge, then fast-forwards local `trunk`.

```
MR arrives → categorize into a LANE → re-parent sender --ref onto origin/main in a scratch worktree
          → push cand/<agent>-<sha>, `gh pr create --base main`, `gh pr merge --squash --auto --delete-branch`
          → GitHub Actions gates it IN PARALLEL with every other in-flight candidate
          → GitHub AUTO-MERGES the PR to origin/main the instant its CI is green (no pr-sync action)
          → pr-sync POLLS (fleet ci-status): PR merged  → FF local `trunk` from origin/main + ack `merged`
                                             CI red     → ack `reject` (failing job + run-log URL); close PR
          → NO local gate, NO combined-tree bisect (a red candidate fails its OWN PR alone, blocks nothing)
```

pr-sync's tick becomes a **scheduler pass**, not a serial gate loop: reap concluded PRs
(merged→FF-trunk+ack / red→reject), then top up in-flight capacity by pushing new candidates from
the queue, respecting per-lane ordering + a global in-flight cap. Dropping the bisect is a direct
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
   file): when a green PR is about to land, re-check it still fast-forward-merges onto the CURRENT
   trunk. If trunk advanced under it since CI started AND the merge no longer FFs cleanly →
   **re-base the candidate on the new trunk, re-push, re-gate** (its CI re-runs), rather than force a
   possibly-broken merge. This is the "re-gate if another landed first" the operator described.
4. A candidate whose lane predicate says it's disjoint from everything currently in flight skips the
   re-check (provably can't collide).

Landing is still a **fast-forward / clean `--no-ff` merge only** — never a conflict resolution by
pr-sync (that stays the author's job, same as today's reject-on-conflict).

**Reap edge case — a CLOSED-but-not-merged PR.** `reap_action(merged, verdict)` today resolves
merged→land, not-merged+red→reject, else→wait. But a candidate PR can also be **CLOSED without
merging and without a red CI** (manually closed, superseded, or its branch deleted) — that reads
`merged=false` + a pending/green verdict → `KeepWaiting` FOREVER, a stuck in-flight slot. I4 must add
the PR `state` as a third reap input: `CLOSED && !MERGED` → treat as a `Reject`-equivalent (ack the MR
so its sender can resend, drop the `ci-dispatch` record, free the slot). `pr_merged_and_verdict`
already fetches `state` via `gh pr view` — extend it to return the closed-not-merged case. (Deferred to
the I4 executor build, not the read-only preview; noted so it isn't lost.)
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
- **I4 — the scheduler executor.** Replace pr-sync's local `gate_subset` land loop with a scheduler
  pass: reap-concluded (PR merged → FF local `trunk` from `origin/main` + `ack merged`; CI red →
  `ack reject` with failing job + run URL, close PR) → top-up in-flight per lane under a global cap.
  Update `fleet/loops/pr-sync.md` + `AGENTS-fleet.md`. (No bisect — a red fails its own PR alone.)
- **I5 — hardening.** In-flight cap tuning, CI timeout → reject-with-timeout, stale-candidate-branch
  GC, the housekeeping-publish path for pr-sync's `issues/` archive (finding 9 above), and remove the
  now-dead local-gate code (`gate_subset`, `gate-batch` bisect) once I4 is proven.

Each increment lands as its own merge-request (per-commit cadence). Trunk-safety invariant holds at
every step: nothing advances trunk except a CI-green candidate.

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
