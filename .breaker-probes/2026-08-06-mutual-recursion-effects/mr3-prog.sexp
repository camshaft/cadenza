(do
  (effect St (op count (-> Unit Int64)))
  (def (solo (: k Int64))
    (if (= k 0) (St.count) (solo (- k 1))))
  (def (main (: n Int64))
    (handle St 0
      ((count (u) s (resume s (+ s 1))))
      (+ (* 10 (solo 4)) (solo 3))))
  (export main))
