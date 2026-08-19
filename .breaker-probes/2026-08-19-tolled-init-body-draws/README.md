# Tolled outer drawn at init AND in the inner body (2026-08-19)

- `hoh7.sexp` — both draws hit the x10000-tolled outer arm: the INIT
  draw's continuation contains the WHOLE inner handle (install + body +
  close) while the BODY draw's contains only the region close; the two
  tolls price captures seven apart (90018 = fold 10*t0 + t0+7 + tolls
  10000*(t0+7) + 10000*t0). Composes hoh6's thread continuity with the
  toll-scope law across the install boundary. PASS x3 at f62a6dc18.
