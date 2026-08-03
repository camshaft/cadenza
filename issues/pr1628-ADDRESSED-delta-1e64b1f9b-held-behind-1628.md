# PR #1628 review comments — fleet/CI-GATED-LANES-DESIGN.md (v-fleet-tooling) — OPEN

https://github.com/camshaft/cadenza/pull/1628 (correct the trunk-advance model in the design doc — the
same #1575 mergeCommit.oid correction, now in the CI-GATED-LANES-DESIGN doc). Two LOW grammar nits.

## 1. Missing verb — "After the reap it `git remote prune origin`" (Copilot, :74) — doc/grammar
> "After the reap it `git remote prune origin` …" is missing a verb, which makes the sentence hard to read.

Add the verb: "After the reap it RUNS `git remote prune origin` …". LOW.

## 2. "git fatals" is slang — use "git fails" (Copilot, :57, also :272) — doc/grammar
> "git fatals" is slang/incorrect grammar. Use "git fails" (or similar) so the doc reads cleanly.

Two sites (:57, :272). Reword "git fatals" → "git fails" / "git errors out". LOWEST/style.
