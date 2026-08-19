# Subtracted tolls drive the answer negative (2026-08-19)

- `pyv4.sexp` — the arm SUBTRACTS its toll: (- (resume ...) (* 100 (+ s
  1))). The unwind folds below zero and keeps going (-290 / -200); an
  unsigned intermediate slot or a clamp anywhere in the unwind path
  shows immediately. All prior post-resume answers were positive — this
  pins the SIGNED range of the unwind arithmetic (companion to pyv2's
  ordered-but-positive difference toll). PASS x3 at f62a6dc18.
