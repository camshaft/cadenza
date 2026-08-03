# PR #1170 review comment — implementation/iterators/src/giter-stepby.cdz (v-iterators)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1170
(PR: "cand: v-iterators — giter-stepby (rebased resend)"). Follows from the #1156 rationale
narrowing — the test name now lags the reframed description.

## Test name `step-by-exhausts-mid-stride` no longer matches description (Copilot, giter-stepby.cdz:91) — naming
> The test name `step-by-exhausts-mid-stride` no longer matches the updated description (this case
> is now framed as a minimal trailing-skip witness). Renaming the test to reflect the behavior under
> test will make the suite easier to scan and keep names consistent with `giter.cdz`'s `step-by-*`
> test naming.

Tidy follow-on to the #1156 rationale fix: rename the test to match its now-reframed purpose
(minimal trailing-skip witness) and the `step-by-*` naming convention in giter.cdz.
