(case "ra3min3 recursion draws both effects, trailing draw on B ONLY"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (race (: k Int64))
              (let ((a (A.next)))
                (let ((b (B.next)))
                  (if (< a b) (race (+ k 1)) k))))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (handle B (+ n 3)
                  ((next () t (resume (+ t 2) (+ t 2))))
                  (let ((steps (race 0)))
                    (+ (* 100 steps) (B.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 12 Int64)))
