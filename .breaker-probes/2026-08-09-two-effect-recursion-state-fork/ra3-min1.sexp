(case "ra3min1 SINGLE-effect recursion + trailing draw (the FIXED #12 shape as control)"
  (input  (do
            (effect A (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((a (A.next)))
                (if (< a 20) (walk (+ k 1)) k)))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (let ((steps (walk 0)))
                  (+ (* 100 steps) (A.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 225 Int64)))
