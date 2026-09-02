(do
  (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))

  (def (mk (: v Int64)) (let ((((. Percent Pct) p) ((. Percent Pct) v))) p))

  (def (main) (mk 50))

  (export main))
