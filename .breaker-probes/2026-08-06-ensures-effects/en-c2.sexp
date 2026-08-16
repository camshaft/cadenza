(do
  (effect St (op bump (-> Unit Int64)))
  (@ (ensures (>= ret 0)) (def (f (: x Int64)) (+ x (St.bump))))
  (def (main (: n Int64))
    (handle St 100
      ((bump (u) s (resume s (+ s 1))))
      (f n)))
  (export main))
