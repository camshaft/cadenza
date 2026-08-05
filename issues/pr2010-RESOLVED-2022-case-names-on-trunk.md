# PR #2010 review — rcdzc effects.rs + tests.rs (v-effects) — MERGED — doc-maintainability [VERIFIED, LOW] (batched)

https://github.com/camshaft/cadenza/pull/2010 (FIX strict-operand face of inner-abort-rollback). Copilot 2
inline, both the SAME class: code comments cite a semantics-corpus LINE NUMBER that shifts as cases are
added. Batched.

## `effects.rs:5239` (id 3711683150) + `tests.rs:66484` (id 3711683174): comments reference `14-eff:1251` / "deep-nested 1251" — a corpus line number that this very PR shifts (Copilot) — doc-maintainability [VERIFIED]
> This comment references a specific line number in the semantics corpus ("14-eff:1251"), but this PR adds
> new cases above and shifts line numbers, so the reference is already stale/misleading. Prefer
> referencing the case name (or a stable identifier) instead of a line number.
> [tests.rs] The test comment refers to "deep-nested 1251" … will drift over time and is already likely
> incorrect. Refer to the stable semantics case name instead.

VERIFIED on trunk: `effects.rs:2465` and `:5237` both cite `14-effects:1251` / `14-eff:1251` ("the
correct-because-unobserved 14-eff:1251 `(+ (A.a) (+ (B.b) (Bail.bail 99)))`"), and `tests.rs:66484` says
"deep-nested 1251 (stays 99 …)". These are LINE-NUMBER references into `spec/semantics/14-effects…sexp`,
and corpus PRs (including this one) insert cases above line 1251, so the number drifts — a reader following
`:1251` today lands on the wrong case. LOW/doc-maintainability. Fix per Copilot: cite the stable CASE NAME
(the `(case "…")` title) instead of the line number — e.g. reference the case by its quoted name so it
survives insertions. (Generalizable authoring note: line-number cross-refs into the corpus rot; case-names
are stable. Same lesson as the corpus churn-count family — worth a standing convention.) v-effects owns
effects.rs + rcdzc/tests.rs. Both LOW, batchable.
