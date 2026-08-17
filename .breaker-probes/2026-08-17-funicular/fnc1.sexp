(case "fnc1 a FUNICULAR of counterbalanced cars — boarding needs car A at the base (else refused showing its position), each run climbs two capped at four where ARRIVAL counts the trip unloads every passenger and swaps the car back to base (a seven-hundred row carrying trip and unloaded headcount), a mid-slope run answers BOTH cars' positions from one field (posA and its derived mirror four-minus-posA), and the seed starts the car mid-slope so one run's first board is refused while its early arrival opens the base for the second"
  (input  (do
            (effect F
              (op board (-> Int64 Int64))
              (op run (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle F (tuple (* (% n 3) 2) (: 0 Int64) (: 0 Int64))
                ((board (k) st
                  (match st
                    ((tuple posA pas trips)
                      (if (= posA 0)
                          (resume (+ (* (+ pas k) 10) k) (tuple posA (+ pas k) trips))
                          (resume (+ (: 900 Int64) posA) st)))))
                 (run () st
                  (match st
                    ((tuple posA pas trips)
                      (if (>= (+ posA 2) 4)
                          (resume (+ (: 700 Int64) (+ (* (+ trips 1) 10) (% pas 10)))
                                  (tuple (: 0 Int64) (: 0 Int64) (+ trips 1)))
                          (resume (+ (* (+ posA 2) 10) (- 4 (+ posA 2)))
                                  (tuple (+ posA 2) pas trips))))))
                 (read () st
                  (match st
                    ((tuple posA pas trips)
                      (resume (+ (* trips 100) (+ (* posA 10) pas)) st)))))
                (let ((a (F.board (: 3 Int64))))
                  (let ((b (F.run)))
                    (let ((c (F.board (: 2 Int64))))
                      (let ((d (F.run)))
                        (let ((f (F.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 902710022022122 Int64))
  (call   main (: 0 Int64)) (output (: 33022902713100 Int64)))
