# cnl1 — canal lock chamber (2026-08-17, tick 1692)

Attack: a DOUBLE-TRIGGER auto-open — the fill arm tests the cap overflow
(`> 9`) and exact-arrival (`= 9`) SEPARATELY, each with a gate-shut inner
test (4 leaves total), and BOTH auto-open leaves produce the identical
700-row + identical rebuild (the same-answer-same-rebuild pair reached by
different predicates — a branch-merge bait where merging is CORRECT, the
mirror of rcy1's must-not-merge). Enter's serve leaf mutates all 3 fields
(count, shut, drain); refuse resumes untouched.

Differential: starting water 6 vs 3: n=10 auto-opens on fill #1 (700) so
enter #1 is SERVED and enter #2 refused; n=0 auto-opens on fill #2 — the
enter attempts are served/refused in OPPOSITE orders (rows mirror:
[700,13,60,906] vs [60,906,700,13]), reads 610 vs 310.

Hand model: n=10 → 7000130609060610; n=0 → 609067000130310 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk e4b91e88b.
