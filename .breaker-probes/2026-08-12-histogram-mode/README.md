# 2026-08-12 histogram + mode (tick 1353, base post-243 trunk)

- `hst1.sexp` — histogram state (Map Int64 Int64) keyed by DECADE BUCKET computed
  in the arm (v/10); obs counts via lookup-match accumulate; mode enumerates via
  Map.to-list and a recursive best-walk (strictly-greater keeps the FIRST-in-
  enumeration bucket on ties — relies on sorted enumeration, cf. mi1/mi2 pins).
  Composes: arm-computed keys + counting + enumeration + recursive fold over the
  dumped list, all through one state thread. Seeds: n=12 spreads 3 buckets
  (mode 1:3 → 13), n=33 stacks one (3:4 → 34). PASS ×3 (1213013/1234034).
