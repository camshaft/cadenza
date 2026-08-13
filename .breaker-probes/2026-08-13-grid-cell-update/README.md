# 2026-08-13 2D grid cell update (tick 1403)

- `grd1.sexp` — (List (List Int64)) grid state: setc rebuilds ONE CELL via
  NESTED List.update (inner row carved by List.at, cell replaced by inner
  update, row reinstalled by outer update — structural sharing must copy only
  the touched spine paths); answers re-sum the WHOLE grid via a two-level
  recursive fold; getc bound-checks both levels (-1 at 9,9). ll1 pins row
  APPEND + reads; the nested-UPDATE write path is new. Seeds shift both set
  values (n / n+5). PASS ×3 (150724121/80010051).
