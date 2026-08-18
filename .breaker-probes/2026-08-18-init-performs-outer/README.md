# Inner INIT performs on the live outer handler (2026-08-18)

- `hoh3.sexp` — the inner handle's INIT expression performs TWO ops on
  the OUTER handler: (handle E (+ (T.levy) (* 10 (T.levy))) ...). The
  levies run while only the outer frame exists (before the inner
  installs), advancing outer state in order (6261: init = 1 + 10*6 = 61,
  draws 61 and 62). Completes the INIT-expression family: hoh1 closed
  inner handle in INIT, hoh2 tolled inner handle in INIT, wnw1 match in
  INIT, hoh3 CROSS-FRAME PERFORMS in INIT. The frame-existence boundary
  (who can serve an INIT-time perform) is the pin. PASS x3 at e1179195f.
