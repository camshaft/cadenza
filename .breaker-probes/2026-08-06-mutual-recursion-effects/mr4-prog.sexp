(do
  (effect St (op count (-> Unit Int64)))
  (def (ev (: k Int64))
    (if (= k 0) (St.count) (od (- k 1))))
  (def (od (: k Int64))
    (if (= k 0) (+ 100 (St.count)) (ev (- k 1))))
  (def (main (: n Int64))
    (handle St 0
      ((count (u) s (resume s (+ s 1))))
      (+ (* 10 (ev 4)) (ev 2))))
  (export main))
