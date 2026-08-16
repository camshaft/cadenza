(case "ra3min4 recursion draws A only, but INSIDE the nested B handle (B unused in recursion), trailing A"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((a (A.next)))
                (if (< a 20) (walk (+ k 1)) k)))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (handle B (+ n 3)
                  ((next () t (resume (+ t 2) (+ t 2))))
                  (let ((steps (walk 0)))
                    (+ (* 100 steps) (A.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 225 Int64)))
