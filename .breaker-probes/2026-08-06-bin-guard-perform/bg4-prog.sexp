(do
  (effect St (op quota (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((quota (u) s (resume s (+ s 1))))
      (match (tuple 7 42)
        ((guard (tuple tag val) (> val (St.quota))) (+ (* 100 tag) val))
        (_other -1))))
  (export main))
