(case "ra3 TRAILING draws on BOTH threads after the race returns — post-recursion state reads on two independent effects (the #12 face doubled)"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (race (: steps Int64))
              (let ((a (A.next)))
                (let ((b (B.next)))
                  (if (< a b) (race (+ steps 1)) steps))))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (handle B (+ n (+ (* 2 (if (< (% n 5) 0) (- 0 (% n 5)) (% n 5))) 3))
                  ((next () t (resume (+ t 2) (+ t 2))))
                  (let ((steps (race 0)))
                    (+ (* 100 steps) (- (A.next) (B.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64))
  (call   main (: 1 Int64)) (output (: 104 Int64))
  (call   main (: -4 Int64)) (output (: 304 Int64)))
