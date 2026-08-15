# lft1 — elevator load limiter (2026-08-15, tick 1566)

(load, refused) state: `board` adds weight answering the load, or REFUSES
with the negated overage (load untouched, refusal counted) when the
seed-shaped capacity (100+5n: 150 vs 100) would be exceeded; `alight`
subtracts clamped at empty; `trips` counts refusals. The same boarding
sequence is refused ONCE on the roomy car (-5 at the third board) and TWICE
on the tight one (-10 then -5 at DIFFERENT boards), the clamp-to-empty fires
only on the tight car (60-70 → 0), and both cars accept the final 45.

Completes the refusal-family: chg1 (reserve floor), vnd1 (change float),
lft1 (capacity ceiling) — three distinct refusal semantics, all pinned.

PASS ×3. **Pool — fills bch1/fzk1/lft1 (11th trio ready).**
