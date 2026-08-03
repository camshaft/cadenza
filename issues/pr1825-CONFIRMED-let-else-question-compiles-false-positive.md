# PR #1825 review comment — cdz-kernel/src/event_ast.rs (v-agent-harness) — MERGED — VERIFIED FALSE POSITIVE

https://github.com/camshaft/cadenza/pull/1825 (MERGED).

## Copilot: `let [..] = expr?.ok_or()? else {}` "won't compile" — FALSE POSITIVE (Copilot, event_ast.rs:350)
> `let ... else` on an expression that already applies `?` won't compile: let-else requires a plain
> expression, not one that early-returns via `?`.
FALSE. `?` is an expression operator that evaluates to the unwrapped value; `let PATTERN = expr? else {}`
is valid — the `?` resolves first, then the (refutable) slice-pattern `[name_f, hash_f]` is matched with
the else-arm as the refutation branch. VERIFIED it compiles: #1825 is MERGED (CI green) and the code is on
trunk (event_ast.rs:344-350) as `let [name_f, hash_f] = a.as_form(...).ok_or(...)? else { return … };`.
No action — Copilot mis-analyzed let-else + `?` interaction. DISMISSED. (Sent v-agent-harness a brief
FYI so they don't churn working code on the bad review.)
