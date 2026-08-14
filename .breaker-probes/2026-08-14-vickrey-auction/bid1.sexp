(case "bid1 a SECOND-PRICE auction tracker — each bid answers the index of the CURRENT leader (a beaten high demotes to second, a middle bid bumps only second), and the closing draws read the winner index and the price he actually pays"
  (input  (do
            (effect A
              (op bid (-> Int64 Int64))
              (op winner (-> Int64))
              (op price (-> Int64)))
            (def (main (: n Int64))
              (handle A (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((bid (v) st
                  (match st
                    ((tuple hi sec w cnt)
                      (if (< hi v)
                          (resume (+ cnt 1) (tuple v hi (+ cnt 1) (+ cnt 1)))
                          (if (< sec v)
                              (resume w (tuple hi v w (+ cnt 1)))
                              (resume w (tuple hi sec w (+ cnt 1))))))))
                 (winner () st
                  (match st ((tuple hi sec w cnt) (resume w st))))
                 (price () st
                  (match st ((tuple hi sec w cnt) (resume sec st)))))
                (let ((a (A.bid (+ n 5))))
                  (let ((b (A.bid 8)))
                    (let ((c (A.bid (+ n 12))))
                      (let ((d (A.bid 17)))
                        (let ((e (A.winner)))
                          (let ((f (A.price)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10103030317 Int64))
  (call   main (: 0 Int64)) (output (: 10203040412 Int64)))
