# Conditional double replay (2026-08-18)

- `dbr4.sexp` — MIXED one-shot/multi-shot in one machine: (if (> s 0)
  <double replay, second wins> <single resume>). Seeds place the
  single-replay frame at different depths: main(10) (s0=1) double-replays
  at BOTH dispatches (141); main(0) single-resumes dispatch 1 then
  double-replays dispatch 2 (110). Extends the dbr family (uniform
  double) with per-frame replay-count divergence — a lowering that
  specializes the arm to a fixed replay count per handler (rather than
  per activation) mixes the paths up. PASS x3 at 3cd560c66.
