# PR #1206 review comments — rcdzc/src/resolve.rs + tests.rs (v-diagnostics)

Mirrored from https://github.com/camshaft/cadenza/pull/1206 (PR: "cand: v-diagnostics — 02664e5e3").

## 1. Rest-binder error message implies only nested-pattern is invalid (Copilot, resolve.rs:1551) — doc/diagnostic
> The rest-binder error message currently implies the only invalid case is a nested pattern; however
> the actual rule is "must be a name or `_`", so other non-name forms (e.g., literals) would also be
> rejected and the message becomes slightly misleading. Consider broadening the wording while keeping
> the nested-pattern guidance.

## 2. `list_form_has_nested_rest` doc says COMPOUND but predicate matches any non-name (Copilot, resolve.rs:3177) — doc/correctness
> The doc comment for `list_form_has_nested_rest` says the post-`..` element is a "COMPOUND" nested
> pattern (list/tuple/ctor), but the implementation treats any non-name node as "nested-rest shape"
> (via `as_name(..).is_none()`). Either tighten the predicate to only match compound patterns, or
> adjust the comment to match the actual behavior.

## 3. `diags_of(name_rest)` evaluated twice per assertion (Copilot, tests.rs:52229, also :52233, :52244) — test efficiency
> This assertion calls `diags_of(name_rest)` twice (once for the check and again for the failure
> message), which needlessly re-runs compilation/diagnostics and can slow the test suite. Capture the
> diagnostics once and reuse it.

Points 1+2 are a doc/behavior-alignment pair (the "must be name or `_`" rule vs the "nested/compound"
wording — pick one framing and make message + predicate + comment agree). Point 3 is the same
`diags_of` double-eval pattern already flagged on #1167 — bind to a local and reuse.
