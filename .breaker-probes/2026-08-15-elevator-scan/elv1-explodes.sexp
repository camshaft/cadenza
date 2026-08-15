(case "elv1 an ELEVATOR scan over floors zero to seven — call registers a request bit answering the pending count, move advances one floor in the travel direction reversing at the extremes and answers floor*10 plus a served flag when it clears a request, and the seed's starting floor decides WHICH calls get served within four moves"
  (input  (do
            (effect E
              (op call (-> Int64 Int64))
              (op move (-> Int64)))
            (def (bits (: b Int64) (: acc Int64))
              (if (= b 0) acc (bits (>> b 1) (+ acc (& b 1)))))
            (def (main (: n Int64))
              (handle E (tuple (% n 8) (: 1 Int64) (: 0 Int64))
                ((call (f) st
                  (match st
                    ((tuple floor d mask)
                      (resume (bits (| mask (<< 1 f)) 0) (tuple floor d (| mask (<< 1 f)))))))
                 (move () st
                  (match st
                    ((tuple floor d mask)
                      (if (< 7 (+ floor d))
                          (if (= (& (>> mask (- floor 1)) 1) 1)
                              (resume (+ (* (- floor 1) 10) 1) (tuple (- floor 1) -1 (& mask (^ (<< 1 (- floor 1)) -1))))
                              (resume (* (- floor 1) 10) (tuple (- floor 1) -1 mask)))
                          (if (< (+ floor d) 0)
                              (if (= (& (>> mask 1) 1) 1)
                                  (resume 11 (tuple 1 1 (& mask (^ (<< 1 1) -1))))
                                  (resume 10 (tuple 1 1 mask)))
                              (if (= (& (>> mask (+ floor d)) 1) 1)
                                  (resume (+ (* (+ floor d) 10) 1) (tuple (+ floor d) d (& mask (^ (<< 1 (+ floor d)) -1))))
                                  (resume (* (+ floor d) 10) (tuple (+ floor d) d mask)))))))))
                (let ((a (E.call 4)))
                  (let ((b (E.call 1)))
                    (let ((c (E.move)))
                      (let ((d (E.move)))
                        (let ((e (E.call 6)))
                          (let ((f (E.move)))
                            (let ((g (E.move)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1023041025061 Int64))
  (call   main (: 0 Int64)) (output (: 1021120023041 Int64)))
