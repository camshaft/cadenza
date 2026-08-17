# ptc1 — portcullis with ratchet pawl (2026-08-17, tick 1683)

Attack: a SEED-DERIVED OP ARGUMENT — the pawl call's argument is itself the
seed-branched expression `(if (> (% n 3) 0) 0 1)` (the INVERSE of the init's
pawl), so the body's dataflow (not just the handler state) branches on the
seed. The release arm's hold branch resumes st untouched while the fall
branch zeroes height and echoes the LOST height (a read-then-clear). Crank
strain grows by `(/ (+ h 2) 4)` — height-dependent accumulation with the
capped branch answering a fixed 80-row.

Differential: initial pawl 1-vs-0 with the mid-run toggle INVERTED per seed:
release #1 holds (804) then release #2 falls (704) on n=10; exactly the
reverse (704 then 800) on n=0 — same op pair, mirrored outcomes, and the
read's pawl bit lands opposite (010 vs 011).

Hand model: n=10 → 20041804004704010; n=0 → 20041704010800011 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 6789dc56e.
