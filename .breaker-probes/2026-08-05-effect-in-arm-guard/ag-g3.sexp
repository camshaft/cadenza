(do
  (effect St (op roll (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((roll (u) s (resume s (+ s 3))))
      (match n
        ((guard v (> (St.roll) 4)) (* v 100))
        (v v))))
  (export main))
