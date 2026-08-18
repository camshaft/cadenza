# Foreign tolled perform whose continuation escapes the inner region (2026-08-18)

- `pyq1.sexp` — the inner E body performs (T.levy) on the TOLLED outer
  handler, so T's continuation spans PAST the inner handle's close:
  it finishes the E body, lets E's toll settle, applies the tenfold
  scaling, and adds the first draw — all inside T-levy#2's resumed
  continuation, before T's own x10000 toll lands (55071 / 35060,
  n=10: verified by the CPS model built with the pysh3 ruling's
  continuation-scope law, cross-checked against actuals BEFORE pinning,
  then confirmed x3). Three tolls across two frames and a region
  boundary — the first probe designed WITH the corrected model from
  the start. PASS x3 at 0c2b86ad3.
- `pyq2.sexp` — the escaping levy as the inner body's LAST form: (+ (* 10
  (E.tick)) (T.levy)). The levy's continuation carries only the addition,
  the inner toll, and the region close (20551 / 10550, CPS-modeled and
  actual-cross-checked before pinning). ORDER BOUNDARY (ladder): the
  MIRROR shape (+ (T.levy) (* 10 (E.tick))) — escaping levy BEFORE an
  inner draw — DECLINES at the tail-resumptive fold, as does the
  both-sides form. So an escaping foreign perform folds only when no
  LATER inner dispatch remains in its continuation... yet pyq1 (levy
  after the inner draw inside a +) folds too — consistent: in both
  passing shapes the levy is the LAST dispatch of the inner body. Face
  added to the fold-boundary flip-watch.
