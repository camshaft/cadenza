# 2026-08-12 symbol-keyed Map state (tick 1346, base post-242 trunk)

- `smy1.sexp` — handler state `(Map Symbol Int64)`: the op takes a SYMBOL argument
  crossing the perform boundary, the arm routes accumulation by interned identity
  (lookup-match accumulate-or-insert), and the same label performed at two different
  call sites lands in ONE bucket (Symbol.of "hot" twice). First Symbol-keyed Map
  anywhere in the corpus (Symbol coverage in 14* is op-result/arg scalars; no
  Map keyed on interned symbols). Two-hop arm-perform chain angle was coverage-killed
  (ti4 in 14b); Char angle blocked (String.scalar-at compile-time only). PASS ×3
  (30413 / 404150).
