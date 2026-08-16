(do
  (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
  (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
  (def (main (: n Int64)) (unwrap (__invariant_construct_Percent n)))
  (export main))
