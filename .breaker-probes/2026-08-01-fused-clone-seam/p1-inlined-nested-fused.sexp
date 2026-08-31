(do
  (def (inner (: r (Result Int64 Int64))) (match r ((Ok w) (+ w 100)) ((Err e) e)))
  (def (f (: c Bool) (: n Int64))
    (match (if c (Some n) (None))
      ((Some v) (inner (if (< v 5) (Ok v) (Err -1))))
      ((None) 0)))
  (def (main) (+ (* (f true 3) 1000000) (+ (* (f true 7) 1000) (f false 9))))
  (export main))
