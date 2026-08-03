# PR #1767 review comment — spec/semantics/.gate-baseline-rust (corpus/v-rust-backend) — CLOSED, no fix-forward

https://github.com/camshaft/cadenza/pull/1767 (CLOSED, mergedAt=null).

## RRB-concat dedup would remove the ONLY rust-baseline line (coverage regression) — but PR CLOSED (Copilot, .gate-baseline-rust:463)
> This hunk removes the only baseline line for the RRB-vector concatenation case in `.gate-baseline-rust`
> (unlike .gate-baseline/-rust-async which keep a copy elsewhere), so the rust gate no longer records this
> case (coverage regression).

VALID risk ON THAT CANDIDATE, but #1767 is CLOSED (mergedAt=null) — it never landed, so no coverage
regression on trunk. VERIFIED: the RRB case is present exactly 1x on trunk's .gate-baseline-rust (correctly
de-duped by a landed PR, not over-deleted). Same closed-superseded pattern as #1755. NO fix-forward — noting
CLOSED so the finding isn't chased as live. (Per the mergedAt-verify rule, checked state before filing.)
