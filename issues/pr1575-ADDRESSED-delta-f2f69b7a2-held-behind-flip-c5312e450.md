# PR #1575 review comments — fleet/loops/pr-sync.md (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1575 (PR: "[v-fleet-tooling] c5312e450").
Updates the pr-sync role loop doc to the CI-gated integration model. Three Copilot points, all
verified.

## 1. REAP description mismatches schedule-pass's actual trunk-advance (Copilot, pr-sync.md:55) — doc/correctness
> The REAP description doesn't match `schedule-pass`'s actual trunk-advance behavior. The executor
> advances trunk by cherry-picking the candidate PR's squash merge commit (`mergeCommit.oid`) onto
> `trunk` (not by cherry-picking `trunk..origin/main`).

VERIFIED against the code: `advance_trunk_for_merged_pr` (xtask/src/fleet.rs:8314) reads the PR's OWN
`mergeCommit.oid` via `gh pr view … --jq .mergeCommit.oid` and `cherry-pick`s THAT onto trunk (:8338,
:8387) — explicitly "NOT origin/main's tip and NOT the whole range" (:8330-8337). But the doc
(pr-sync.md:52) says "advance `trunk` to `origin/main` (cherry-picks `trunk..origin/main` …)". This is
the SAME advance-trunk-by-mergeCommit model the #1532 fix established — the doc lags the code. Fix the
REAP bullet to describe cherry-picking the reaped PR's `mergeCommit.oid`, matching #1532. SUBSTANTIVE
(a future debugger reads the doc as the contract).

## 2. Reject-ack "PR/run link" doesn't match actual short-reason-with-PR-number ack (Copilot, pr-sync.md:28) — doc
> This section says reject acks should include a "PR/run link", but `schedule-pass` currently acks
> rejections with a short reason that references the candidate PR number (no URL).

The doc (pr-sync.md:28) says a reject ack is "a SHORT reason + the PR/run link". If the executor's ack
is a short reason + PR *number* (no URL), reconcile — either soften the doc to "PR number/reference"
or have the ack emit the URL. Worth a quick check against the actual `fleet ack reject` body. LOW.

## 3. Time-stamped "operator-greenlit + smoke-tested 2026-08-03" will go stale in a role doc (Copilot, pr-sync.md:49) — doc/durability
> The parenthetical "operator-greenlit + smoke-tested 2026-08-03" is a time-stamped status claim that
> will go stale in a role loop doc. Point at the stable design/contract reference instead.

VERIFIED (pr-sync.md:48): "the CI-gated executor, operator-greenlit + smoke-tested 2026-08-03;
replaces the old …". Same durability pattern as #1554/#1573 — a dated status line in a living role
doc. Point at the stable design ref (CI-GATED-LANES-DESIGN.md) instead of the date. LOW.
