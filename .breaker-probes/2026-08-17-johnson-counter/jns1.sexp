(case "jns1 a JOHNSON twisted-ring counter on TWO BOOL state slots and a pulse count — pulse shifts q into p and the NEGATION of p into q answering a bit-packed readout of both new flags plus an equality bit and the count, align SWAPS the flags only when they differ tagging swap and hold answers differently and touching no count, and the seed loads the ring so one run walks the four-phase cycle from inside while the other enters at the seam"
  (input  (do
            (effect L
              (op pulse (-> Int64))
              (op align (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (= (% n 3) 1) false (: 0 Int64))
                ((pulse () st
                  (match st
                    ((tuple p q cnt)
                      (resume (+ (* (+ (+ (if q (: 4 Int64) (: 0 Int64))
                                          (if (not p) (: 2 Int64) (: 0 Int64)))
                                       (if (= q (not p)) (: 1 Int64) (: 0 Int64)))
                                    10)
                                 (% (+ cnt 1) 10))
                              (tuple q (not p) (+ cnt 1))))))
                 (align () st
                  (match st
                    ((tuple p q cnt)
                      (if (= p q)
                          (resume (+ (: 30 Int64) (% cnt 10)) st)
                          (resume (+ (: 70 Int64) (% cnt 10)) (tuple q p cnt)))))))
                (let ((a (L.pulse)))
                  (let ((b (L.pulse)))
                    (let ((c (L.align)))
                      (let ((d (L.pulse)))
                        (let ((e (L.pulse)))
                          (let ((f (L.pulse)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 112272132475 Int64))
  (call   main (: 0 Int64)) (output (: 217232431425 Int64)))
