(case "trb1 the TRIBONACCI recurrence with a window readout — step rolls (a,b,c) to (b,c,a+b+c) answering the new term, peek answers the live window sum without advancing, and the seed shapes the leading term so the whole orbit shifts while fib1's two-term twin stays covered separately"
  (input  (do
            (effect T
              (op step (-> Int64))
              (op peek (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (% n 3) (: 1 Int64) (: 1 Int64))
                ((step () st
                  (match st
                    ((tuple a b c)
                      (resume (+ a (+ b c)) (tuple b c (+ a (+ b c)))))))
                 (peek () st
                  (match st
                    ((tuple a b c) (resume (+ a (+ b c)) st)))))
                (let ((p (T.step)))
                  (let ((q (T.step)))
                    (let ((r (T.peek)))
                      (let ((s (T.step)))
                        (let ((t (T.step)))
                          (let ((u (T.peek)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 p) q)) r)) s)) t)) u)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 30509091731 Int64))
  (call   main (: 0 Int64)) (output (: 20407071324 Int64)))
