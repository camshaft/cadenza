(do
  (effect A (op tick (-> Unit Int64)))
  (effect B (op tock (-> Unit Int64)))
  (def (main (: n Int64))
    (+ (* 100 (handle A n
                ((tick (u) s (resume s (+ s 1))))
                (handle B (A.tick)
                  ((tock (u) t (resume t (+ t (A.tick)))))
                  (+ (B.tock) (B.tock)))))
       (handle A 50
         ((tick (u) s (resume s (+ s 1))))
         (A.tick))))
  (export main))
