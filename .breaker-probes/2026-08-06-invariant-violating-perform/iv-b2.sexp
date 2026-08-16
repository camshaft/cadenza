(do
  (effect St (op next (-> Unit Int64)))
  (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
  (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
  (def (main (: n Int64))
    (handle St 42
      ((next (u) s (resume s (+ s 1))))
      (let ((v (St.next)))
        (unwrap (__invariant_construct_Percent v)))))
  (export main))
