(do
  (def (g (: v Int64)) (if (< v 5) (Ok (* v 11)) (Err (- 0 v))))
  (def (f (: c Bool) (: n Int64))
    (match (if c (Some n) (None))
      ((Some v) (match (g v) ((Ok v) (+ v 100)) ((Err e) (- e 100))))
      ((None) 0)))
  (def (main) (+ (* (f true 3) 1000000) (+ (* (f true 7) 1000) (f false 9))))
  (export main))
