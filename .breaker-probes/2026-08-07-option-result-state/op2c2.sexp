(do
  (effect O (op get (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle O n
      ((get (k) s (if (> k s) (resume (- k s) (+ s 1)) (resume -100 (+ s 1)))))
      (+ (O.get 10) (* 10 (O.get 0)))))
  (export main))
