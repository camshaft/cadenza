(do
  (effect St (op next (-> Unit Int64)))
  (def (run (: seed Int64) (: mul Int64))
    (handle St seed
      ((next (u) s (resume s (+ s mul))))
      (+ (St.next) (St.next))))
  (def (main (: n Int64))
    (run (run n 1) 10))
  (export main))
