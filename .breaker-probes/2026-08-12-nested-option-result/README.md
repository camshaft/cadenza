# 2026-08-12 nested-Option op result (tick 1328, base 0074ceca0)

- `noo1.sexp` — op result type `Option (Option Int64)` crossing the resume boundary:
  the arm classifies the descending state into None / Some None / Some (Some s); the
  body's closure runs a nested match distinguishing all three in one run (seed 1 hits
  SS/SN/N, seed 2 hits SS/SS/SN). First nested-Option through a handler dispatch in 14*
  (only prior nested-Option lives in 07-type-system annotation cases). Explicit
  `(: (None unit) ...)` annotations at both nesting depths. PASS ×3.
