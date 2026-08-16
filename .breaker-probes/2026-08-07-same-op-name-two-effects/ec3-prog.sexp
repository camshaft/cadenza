(do
  (effect A (op get (-> Unit Int64)))
  (effect B (op get (-> Unit Int64)))
  (def (main (: n Int64))
    (handle A n
      ((get (u) s (resume s (+ s 1))))
      (handle B 100
        ((get (u) t (resume t (+ t 10))))
        (+ (* 10 (A.get)) (B.get)))))
  (export main))
