# Double replay under a post-resume toll (2026-08-18)

- `pyd1.sexp` — the dbr and pyr families composed: (+ (do (resume ...)
  (resume ...)) (* 1000 (+ s 1))). The toll fires ONCE per dispatch on
  the do's value (the second replay's outcome), NOT once per replay —
  main(10): replay1 discarded, replay2 gives 11, toll +2000 -> 2011.
  A lowering that runs the toll per replay would answer 2011+1000·k
  extra; one that tolls the FIRST replay's value answers 2001. Single-
  perform body keeps the replay tree linear. PASS x3 at 29f934387.
