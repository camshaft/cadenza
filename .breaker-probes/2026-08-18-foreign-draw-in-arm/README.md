# Foreign draw inside the inner arm's answer (2026-08-18)

- `hoh4.sexp` — the inner B arm draws OUTER F while building its resume
  answer: (step () s (resume (+ (* s 10) (F.draw)) (+ s 1))). The outer
  F thread advances once for the body's opening draw (1, ->8) and once
  inside B's dispatch (8, ->15): the arm-side draw sees the state the
  body-side draw left (1801 = 1 + 100*(10*s0+8), CPS-modeled). The
  xhs family pinned mid-arm foreign performs pre-resume in SEQUENCE
  position; hoh4 pins one INSIDE the answer expression with the ordering
  observable through the shared outer thread. PASS x3 at 5ae07931d.
- `hoh5.sexp` — REPEATED foreign draws: both inner dispatches fold an
  outer draw into their answers; the outer ladder climbs by sevens
  ACROSS inner dispatches (t0, t0+7) while the inner doubles (1, 2).
  Both second rungs land in the x1000 addend (28011 / 27010). Either
  thread resetting between dispatches collapses a distinct digit — the
  distinct-effect sibling of pysh5's same-effect dual ladders. PASS x3
  at 67ef1f754.
