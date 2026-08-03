# PR #1130 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1130
(PR: "cand: v-fleet-tooling — next stack commit (priority #1)").

## Doc says `--json state,mergedAt` but code requests only `state` (Copilot, fleet.rs:7028, also :7031) — doc
> The doc comment says `gh pr view <n> --json state,mergedAt`, but the implementation only requests
> `state`. Either include `mergedAt` in the command or update the comment so it matches what's
> actually executed (helps keep `schedule-plan` behavior/documentation consistent).

Doc-vs-code drift on the schedule-plan `gh` invocation — either add `mergedAt` to the `--json` field
list (if it's needed) or drop it from the comment.
