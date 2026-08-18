# Triple sequential replay (2026-08-18)

- `dbr5.sexp` — three resumes in one do, first two outcomes discarded:
  extends dbr1's second-wins law to N-TH-WINS (last replay's value is the
  arm's value). Each replay shifts by ten, so stopping at two replays or
  returning an earlier replay's value is off by a fixed decade (21/20 =
  s+20). Single-perform body keeps the replay count linear at 3. PASS x3
  at 29f934387.
