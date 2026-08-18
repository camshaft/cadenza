# Double replay under a post-resume toll (2026-08-18)

- `pyd1.sexp` — the dbr and pyr families composed: (+ (do (resume ...)
  (resume ...)) (* 1000 (+ s 1))). The toll fires ONCE per dispatch on
  the do's value (the second replay's outcome), NOT once per replay —
  main(10): replay1 discarded, replay2 gives 11, toll +2000 -> 2011.
  A lowering that runs the toll per replay would answer 2011+1000·k
  extra; one that tolls the FIRST replay's value answers 2001. Single-
  perform body keeps the replay tree linear. PASS x3 at 29f934387.
- `pyd2.sexp` — the pre-suspend binding survives BOTH replays: t bound
  before either resume, consumed once after the second (211 / 110 =
  (s+10) + 100*(s+1)). A slot refreshed per replay or dropped by the
  discard would misprice the hundreds. Composes pyw1 (binding rides one
  suspend) with the multi-shot machinery (rides TWO suspend/replay
  cycles + a discard). PASS x3 at 942944f3f.
