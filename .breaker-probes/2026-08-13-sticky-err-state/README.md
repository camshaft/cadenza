# 2026-08-13 sticky-Err Result state (tick 1374)

- `stk2.sexp` — handler state `(Result Int64 Int64)` as a STICKY failure machine:
  Ok pushes accumulate; the over-20 sum answers the raw sum but installs Err(sum)
  as an ABSORBING state (later pushes answer the negated code, state unchanged);
  reset reports which mode it found (0/1) and restores Ok 0. Both seeds cross the
  threshold at different dispatches (n=3 sticks on push-3, so its pre-stick answer
  21 flows out; n=10 sticks on push-2 and push-3 answers -27 from inside Err).
  Result-typed handler STATE with variant-dependent arm behavior + state-restore
  op — rsw1/rsl1 pin Result as op RESULTS, not as the threaded state. PASS ×3
  (12207112 / 19272312).
