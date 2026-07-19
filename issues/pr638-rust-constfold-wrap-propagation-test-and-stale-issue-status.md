# pr638 — rust const-fold wrap: missing arith-propagation test pin + stale PENDING-MERGE issue status (2 Copilot)

Mirrored from GitHub PR #638 review comments (Copilot). Both VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/638 (2-MR publish batch) — v-rust-backend territory.

## #1 — id 3610830124 (rcdzc/src/backend/rust/tests.rs:145) — test coverage gap [test-coverage]
> The new const-wrap sign-extension regression test only exercises `(.wrap)` directly; it doesn't cover the
> previously observed propagation into const-folded arithmetic (e.g.
> `(+ ((. (Int 4) wrap) 8) ((. (Int 4) wrap) 1))` folding to the wrong/out-of-range value). Adding a
> propagation pin would protect against regressions where `.wrap` is correct in isolation but arithmetic
> folding reintroduces the issue.

VERIFIED: the test (rust/tests.rs ~134) loops direct `(. (Int W) wrap) input` cases + unsigned/machine-width
CONTROLs — but no `(+ ((.wrap) ...) ((.wrap) ...))` arith-folding case. The issue note itself (see #2 file)
calls the "arith-propagated (range-escaping) faces" the original miscompile shape. A propagation pin guards
the exact regression that motivated the fix. Legit test-hardening (not a bug — the fix is in).

## #2 — id 3610830145 (issues/adv-rust-constfold-unusual-width-wrap-no-sign-extend.RESOLVED-PENDING-MERGE.md:78) — stale status
> This issue note still says the fix is "PENDING MERGE"/"Still PENDING MERGE", but this PR is an integrator
> publish onto `main` and includes the rust-backend fix + regression test. Update the status to reflect that
> it has landed (and, if applicable, rename the file to drop `RESOLVED-PENDING-MERGE`).

VERIFIED: file is still `...RESOLVED-PENDING-MERGE.md` and body (lines 48/76) says "PENDING MERGE"/"Still
PENDING MERGE". This PR publishes the fix + test onto main, so on land the status is stale — rename to
`.RESOLVED` + drop the PENDING-MERGE wording. Housekeeping.

## Owner
Both v-rust-backend (owns the rust const-fold fix + this issue note). #1 = add a propagation test pin;
#2 = rename/destamp their own issue file now it's landed. Fold together.
