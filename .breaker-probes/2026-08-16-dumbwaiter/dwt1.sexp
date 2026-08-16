(case "dwt1 a DUMBWAITER between three floors with a twelve-unit weight limit — send REFUSES an overweight load (nine-hundred plus the standing load, nothing moves) else loads and travels tallying the floor distance, dump empties at the current floor, report packs trips floor and a loaded flag, and the seed weights the SECOND send so one run refuses it (skewing every later row) while the other carries it"
  (input  (do
            (effect D
              (op send (-> Int64 Int64 Int64))
              (op dump (-> Int64))
              (op report (-> Int64)))
            (def (dist (: a Int64) (: b Int64))
              (if (> a b) (- a b) (- b a)))
            (def (main (: n Int64))
              (handle D (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((send (w dest) st
                  (match st
                    ((tuple floor load trips)
                      (if (> (+ load w) 12)
                          (resume (+ (: 900 Int64) load) st)
                          (resume (+ (* dest 100) (+ load w))
                                  (tuple dest (+ load w) (+ trips (dist dest floor))))))))
                 (dump () st
                  (match st
                    ((tuple floor load trips)
                      (resume (+ (* load 10) floor) (tuple floor (: 0 Int64) trips)))))
                 (report () st
                  (match st
                    ((tuple floor load trips)
                      (resume (+ (* trips 100) (+ (* floor 10) (if (> load 0) 1 0))) st)))))
                (let ((a (D.send (: 7 Int64) (: 2 Int64))))
                  (let ((b (D.send (+ (: 5 Int64) (% n 3)) (: 1 Int64))))
                    (let ((c (D.dump)))
                      (let ((d (D.send (: 9 Int64) (: 2 Int64))))
                        (let ((f (D.report)))
                          (+ (* 10000 (+ (* 10000 (+ (* 10000 (+ (* 10000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2070907007202090221 Int64))
  (call   main (: 0 Int64)) (output (: 2070112012102090421 Int64)))
