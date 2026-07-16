# PR #415: .gate-baseline-rust has 10 duplicate descriptions (merge=union amplifies)
Copilot on merged PR #415 (comments 3591538818 + 3591538838). Verified: awk uniq -d on
spec/semantics/.gate-baseline-rust shows 10 dup descriptions (all Ast.Float cases, each twice, both
`pass` — benign now, but gate --check keys by description so a future todo+pass would silently mask).
Root: merge=union gitattribute (both merge sides append). Extends the pr410/pr412 .gate-baseline dup
issue (that one I fixed: 9dca96722).
ROUTING: durable fix (gate --save dedup + gate --check REJECT dupes + reconsider merge=union) →
v-fleet-tooling; immediate rust-baseline dedup regen → v-core-opt.
