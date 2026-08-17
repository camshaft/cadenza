# Organ stops, state-indexed argument select (2026-08-17)

- `org1.sexp` — a 3-arg op whose arm lets the STATE choose WHICH argument is
  consumed: (let ((v (if (= sel 0) a (if (= sel 1) b c)))) — sweep found ZERO
  3-arg arms with branchy per-arg use (the six existing 3-arg ops all fold
  their args uniformly). The selector steps +2 mod 3 per chord (a rotating
  rank), and swell re-aims it by the played total (selector = f(accumulator),
  the cross-field write) while echoing the new aim back. Both runs walk all
  three selector values; 6/6 rows diverge across seeds. First swell draft
  (sel = played%3) CONVERGED the runs after one dispatch (2/6 divergent) —
  additive re-aim (sel+played)%3 preserves the seed's phase. PASS x3 at
  19aefaeba.
