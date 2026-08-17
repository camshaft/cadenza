# dvb1 — diving board judge panel (2026-08-17, tick 1688)

Attack: a NEGATIVE-FEEDBACK score — each raw score is `(- (* d 3) (/ last 2))`
where `last` is the previous raw (judges grade against the standing mark), so
every answer feeds the next dispatch's compound. The raw compound appears x5
across the 2-leaf arm (test, both answers, both rebuilds' last+total). The
streak resets vs extends on the pre/post comparison `(> raw last)`.

Envelope iterations: x7-compound 3-leaf at 4 dispatches declined; the same at
3 dispatches STILL declined (the floor branch's extra leaf was the tipping
weight); dropping the (unreachable-with-these-inputs) floor to a 2-leaf arm
passes at 3. Confirms pnb1's finding: repetition x leaves is the joint load,
and one leaf can be the margin.

Differential: opening mark 8 vs 4: n=10's first dive scores 5 (non-improving,
50) vs n=0's 7 (improving, 71) — the feedback then drags every later score
(reads 1610 vs 1820, streaks diverge at every row).

Hand model: n=10 → 501010101610; n=0 → 710920201820 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk cde130bab.
