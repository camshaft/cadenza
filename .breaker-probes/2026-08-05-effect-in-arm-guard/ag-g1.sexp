(do
  (effect St (op roll (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((roll (u) s (resume s (+ s 3))))
      (match (St.roll)
        ((guard v (> v 6)) (* v 100))
        (v v))))
  (export main))
