(do
  (effect A (op tick (-> Int64)))
  (def (drive (: d Int64))
    (if (<= d 0) 0 (+ (A.tick) (drive (- d 1)))))
  (def (main (: n Int64))
    (handle A (% n 3)
      ((tick () s (resume (+ s 1) (+ s 1))))
      (drive n)))
  (export main))
