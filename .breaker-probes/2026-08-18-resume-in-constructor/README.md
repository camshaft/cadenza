# Resume inside a tuple constructor (2026-08-18)

- `pyc1.sexp` — (match (tuple (resume ...) (+ s 1)) ((tuple r w) ...)).
  The resume value reaches its consumer through CONSTRUCTION + PATTERN
  BINDING rather than a direct let/match binder — a heap round-trip that
  neither 6c52dbc3c (let-init) nor e6eb3831b (match-scrutinee) obviously
  covers, yet it FOLDS: the tuple constructor with a resume field is
  apparently rewritten by the same generic path. Notable as the passing
  side of the binder ladder's outer edge; if a future refold change
  regresses constructor-position resumes, this is the lock (176/129).
  PASS x3 at 600e3f74f.
