(do
  (effect A (op run (-> Int64 Int64)))
  (effect B (op beat (-> Int64)))
  (def (main (: n Int64))
    (handle B (: 0 Int64)
      ((beat () bs (resume (+ bs 1) (+ bs 1))))
      (handle A (% n 3)
        ((run (d) s
          (if (<= d 0)
              (resume s s)
              (let ((k (B.beat)))
                (resume (+ (* s 10) (A.run (- d 1))) (+ s k))))))
        (A.run n))))
  (export main))
