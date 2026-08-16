(do
  (effect O (op get (-> Int64 (Option Int64))))
  (def (main (: n Int64))
    (handle O n
      ((get (k) s (resume (if (> k s) (Some (- k s)) (None)) (+ s 1))))
      (+ (match (O.get 10) ((Some d) d) ((None) -100))
         (* 10 (match (O.get 0) ((Some d) d) ((None) -100))))))
  (export main))
