(case "tns1 a RACE TO FOUR — each rally point goes to player A when the stroke plus the seed divides by three, the answer packs a century flag on the winner's fourth point, play AFTER the race is decided answers an absorbing 99 without touching the score, and the closing read packs both tallies; the seed decides the winner AND whether the sixth rally is live or dead"
  (input  (do
            (effect G
              (op rally (-> Int64 Int64))
              (op score (-> Int64)))
            (def (main (: n Int64))
              (handle G (tuple (: 0 Int64) (: 0 Int64))
                ((rally (x) st
                  (match st
                    ((tuple pa pb)
                      (if (= (* (- 4 pa) (- 4 pb)) 0)
                          (resume (: 99 Int64) st)
                          (if (= (% (+ x n) 3) 0)
                              (resume (+ (* 100 (/ (+ pa 1) 4)) (+ (* (+ pa 1) 10) pb))
                                      (tuple (+ pa 1) pb))
                              (resume (+ (* 100 (/ (+ pb 1) 4)) (+ (* pa 10) (+ pb 1)))
                                      (tuple pa (+ pb 1))))))))
                 (score () st
                  (match st ((tuple pa pb) (resume (+ (* pa 10) pb) st)))))
                (let ((a (G.rally (: 1 Int64))))
                  (let ((b (G.rally (: 2 Int64))))
                    (let ((c (G.rally (: 3 Int64))))
                      (let ((d (G.rally (: 4 Int64))))
                        (let ((e (G.rally (: 5 Int64))))
                          (let ((f (G.rally (: 6 Int64))))
                            (let ((g (G.score)))
                              (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1011012013023124024 Int64))
  (call   main (: 0 Int64)) (output (: 1002012013114099014 Int64)))
