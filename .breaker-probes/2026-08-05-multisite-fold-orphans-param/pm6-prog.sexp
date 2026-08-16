(do
  (effect St (op price (-> Int64 Int64)))
  (def (main)
    (handle St 0
      ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
      (+ (St.price 1) (+ (St.price 7) (St.price 2)))))
  (export main))
