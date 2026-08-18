# Replays diverging only in the answer (2026-08-18)

- `pyz3.sexp` — both resumes thread the SAME next-state, answers one
  apart: (do (resume (* s 10) (+ s 1)) (resume (+ (* s 10) 1) (+ s 1))).
  The surviving replay's +1 signature appears at BOTH depths of the
  two-perform body (2111 / 1101, CPS-modeled). Kills two collapse
  lowerings: same-state replays merged into one, or first-answer-wins.
  Complements dbr6 (state-only divergence) — the dbr family now pins
  answer-only, state-only, and both-diverge replay pairs. Bank note:
  abort-or-double-replay (if s>1 abort else double replay) DECLINES at
  the fold boundary (pya1-class, /tmp ladder). PASS x3 at 5ae07931d.
