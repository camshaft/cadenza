(case "lft1 an ELEVATOR load limiter — board adds the passenger's weight answering the load or REFUSING with the negated overage when the seed-shaped capacity would be exceeded (the load untouched), alight subtracts clamped at empty, trips counts refusals, and the same boarding sequence is refused ONCE on the roomy car and TWICE on the tight one with different overage readings"
  (input  (do
            (effect L
              (op board (-> Int64 Int64))
              (op alight (-> Int64 Int64))
              (op trips (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((board (w) st
                  (match st
                    ((tuple load refused)
                      (if (< (+ 100 (* n 5)) (+ load w))
                          (resume (- (+ 100 (* n 5)) (+ load w)) (tuple load (+ refused 1)))
                          (resume (+ load w) (tuple (+ load w) refused))))))
                 (alight (w) st
                  (match st
                    ((tuple load refused)
                      (if (< load w)
                          (resume 0 (tuple 0 refused))
                          (resume (- load w) (tuple (- load w) refused))))))
                 (trips () st
                  (match st ((tuple load refused) (resume refused st)))))
                (let ((a (L.board 60)))
                  (let ((b (L.board 50)))
                    (let ((c (L.board 45)))
                      (let ((d (L.alight 70)))
                        (let ((e (L.board 45)))
                          (let ((f (L.trips)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 610995408501 Int64))
  (call   main (: 0 Int64)) (output (: 598995004502 Int64)))
