# Foreign levy in each replay's answer argument (2026-08-18)

- `dbf1.sexp` — both sequential resumes levy the outer handler while
  building their answers: (do (resume (+ s (T.levy)) ...) (resume (+ s
  (T.levy)) ...)). The outer counter advances TWICE per dispatch (once
  per replay), and the surviving second replay carries the SECOND levy's
  value (7 = 1 + t=6 for seed 10) — proving the discarded replay's levy
  still fired (skipping it would give 1 + t=1... no: would give second
  levy AT t0+0 = 1+1=2... model: skipping levy #1 means levy #2 sees
  t0 -> answer 1+t0, vs correct 1+t0+5). Composes dbr (multi-shot) with
  pyt2 (dispatch-order foreign perform): effects in DISCARDED replays are
  NOT rolled back. PASS x3 at 29f934387. (Original id dbt1 collided with
  the amortization case — free-id grep caught it.)
