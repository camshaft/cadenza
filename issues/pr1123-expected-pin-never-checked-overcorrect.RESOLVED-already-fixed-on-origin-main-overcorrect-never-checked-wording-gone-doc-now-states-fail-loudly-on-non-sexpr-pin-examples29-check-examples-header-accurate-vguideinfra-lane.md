# PR #1123 review comments — guide/src/playground/examples.ts + guide/scripts/check-examples.mjs (v-guide-infra)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1123
(PR: "cand: v-guide-infra — check-examples+playground (resend)"). This is the follow-up to my
#1098 note — the reworded doc/error now OVERCORRECTS in the other direction.

## Doc/error says a non-sexpr `expected` pin is "never checked", but a sexpr pass always runs (Copilot, examples.ts:30 + check-examples.mjs:465) — doc/correctness
> [examples.ts:30] The doc comment says an `expected` pin on a non-`sexpr` playground example would
> "silently never be checked", but `check-examples` always runs a `sexpr` pass (authored or toggle)
> and would still compare `expected` there. If the intent is to forbid `expected` on ML-authored
> examples because it would only be checked against the *rendered* s-expr toggle output, the comment
> should say that explicitly (and point at the enforcement in check-examples).
> [check-examples.mjs:465] The thrown error message claims a non-`sexpr` playground `expected` pin is
> "never checked", but the harness still runs a `sexpr` pass via the toggle path and would compare
> `expected` there. If the policy is to forbid `expected` on ML-authored playground examples because
> it would only be checked against the *rendered* s-expr toggle output (and thus be more brittle),
> the comment/error should state that reason instead of implying it's unasserted.

Follow-through on #1098: the original doc understated (said `expected` was asserted everywhere); the
rewrite now overstates the other way (says it's "never checked"). Neither is accurate — a sexpr pass
(authored or toggle) DOES compare `expected`. State the real policy: `expected` on an ML-authored
example is only checked against the *rendered* s-expr toggle output (more brittle), which is why it's
forbidden — and point at the enforcement site.
