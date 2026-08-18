# Double replay then tombstone (2026-08-18)

- `tmb3.sexp` — BOTH replays discarded, a state-keyed tombstone answers:
  (do (resume s ...) (resume (+ s 10) ...) (+ (* s 100) 7)). Neither
  replay's value survives yet both run the body (107 / 7 = s*100+7).
  Completes the replay-consumption spectrum: n-th-wins (dbr5), both
  consumed (dbr3), NEITHER consumed (tmb3) — the tombstone composes
  tmb1's discard law with the multi-shot machinery. PASS x3 at 67ef1f754.
- `tmb4.sexp` — a FOREIGN levy feeds the DISCARDED replay's answer:
  (do (resume (T.levy) ...) tombstone). The levy fires and advances the
  outer thread even though its value flowed only into abandoned work —
  the outer body's later levy reads +5 (1706 / 1705). The value-flow
  refinement of dbf1: effects-not-rolled-back holds even when the
  effect's VALUE lands exclusively in discarded dataflow (vs pyt6's
  PURE trap in the same position, which was elided — effect vs purity,
  not liveness, decides). PASS x3 at 8575e9099.
