(do
  (effect St (op price (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle St 0
      ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
      (+ n (St.price 7))))
  (export main))
