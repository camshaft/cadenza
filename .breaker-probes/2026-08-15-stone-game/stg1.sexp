(case "stg1 a STONE-TAKING game — take removes up to k stones clamped at the pile answering pile-times-ten plus whose turn it was, the player landing the pile at zero answers a hundred-plus-their-id, and the smaller pile ends three moves early so its tail is drained-pile wins for alternating players"
  (input  (do
            (effect G (op take (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G (tuple (+ n 11) (: 0 Int64))
                ((take (k) st
                  (match st
                    ((tuple pile turn)
                      (if (< pile (+ k 1))
                          (resume (+ 100 turn) (tuple 0 (- 1 turn)))
                          (resume (+ (* (- pile k) 10) turn) (tuple (- pile k) (- 1 turn))))))))
                (let ((a (G.take 3)))
                  (let ((b (G.take 4)))
                    (let ((c (G.take 2)))
                      (let ((d (G.take 3)))
                        (let ((e (G.take 4)))
                          (let ((f (G.take 5)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 180141120091050101 Int64))
  (call   main (: 0 Int64)) (output (: 80041020101100101 Int64)))
