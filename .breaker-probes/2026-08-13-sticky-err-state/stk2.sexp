(case "stk2 a STICKY-ERR Result state — pushes accumulate while Ok, the over-20 sum flips the state to an absorbing Err whose pushes answer the negated code, reset reports-and-restores"
  (input  (do
            (effect S
              (op push (-> Int64 Int64))
              (op reset (-> Int64)))
            (def (main (: n Int64))
              (handle S (: (Ok n) (Result Int64 Int64))
                ((push (v) st
                  (match st
                    ((Ok s)
                      (let ((s2 (+ s v)))
                        (resume s2 (if (> s2 20) (Err s2) (Ok s2)))))
                    ((Err e) (resume (- 0 e) st))))
                 (reset () st
                  (resume (match st ((Ok _s) 0) ((Err _e) 1))
                          (: (Ok 0) (Result Int64 Int64)))))
                (let ((a (S.push 9)))
                  (let ((b (S.push 8)))
                    (let ((c (S.push 1)))
                      (let ((d (S.reset)))
                        (let ((e (S.push 2)))
                          (+ (* 10 (+ (* 10 (+ (* 100 (+ (* 100 a) b)) (+ c 50))) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 12207112 Int64))
  (call   main (: 10 Int64)) (output (: 19272312 Int64)))
