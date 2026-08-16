(do
  (effect St (op quota (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((quota (u) s (resume s (+ s 1))))
      (match 42
        ((guard v (> v (St.quota))) (* v 10))
        (_other -1))))
  (export main))
