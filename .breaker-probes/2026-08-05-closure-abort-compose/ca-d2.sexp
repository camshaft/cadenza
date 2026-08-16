(do
  (effect St (op mk (-> Unit (Tuple (-> Int64 Int64) Int64))) (op halt (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((mk (u) s (resume (tuple (fn ((: x Int64)) (+ x s)) 0) s))
       (halt (u) s s))
      (match (St.mk)
        ((tuple f _z)
          (do (St.halt)
              (f 100))))))
  (export main))
