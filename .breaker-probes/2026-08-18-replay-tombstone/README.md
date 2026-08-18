# Double replay then tombstone (2026-08-18)

- `tmb3.sexp` — BOTH replays discarded, a state-keyed tombstone answers:
  (do (resume s ...) (resume (+ s 10) ...) (+ (* s 100) 7)). Neither
  replay's value survives yet both run the body (107 / 7 = s*100+7).
  Completes the replay-consumption spectrum: n-th-wins (dbr5), both
  consumed (dbr3), NEITHER consumed (tmb3) — the tombstone composes
  tmb1's discard law with the multi-shot machinery. PASS x3 at 67ef1f754.
