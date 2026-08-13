# 2026-08-13 nested-Option STATE ladder (tick 1399)

- `oos1.sexp` — the handler STATE is `(Option (Option Int64))` walked through the
  full three-rung ladder BOTH directions: None → Some None → Some (Some v) →
  (wrap to) None. advance climbs one rung per dispatch (installing annotated
  None forms at two depths); classify decodes all three inhabitants between
  steps. noo1 pinned nested-Option as an op RESULT (one-way, arm→body); this
  threads it as STATE with per-dispatch re-matching and the wrap-around
  transition. Payload from the SECOND adv (v=n) survives into the third cls
  (30+n). PASS ×3 (10102350000/10102370000).
