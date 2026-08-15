(case "lky1 a LEAKY-BUCKET meter — arrive fills toward the seed-shaped capacity answering only the OVERFLOW spill (level clamps at the brim), drain leaks five clamped at empty answering the new level, and the small bucket spills twice where the large one never does"
  (input  (do
            (effect K
              (op arrive (-> Int64 Int64))
              (op drain (-> Int64)))
            (def (main (: n Int64))
              (handle K (: 0 Int64)
                ((arrive (v) level
                  (if (< (+ n 8) (+ level v))
                      (resume (- (+ level v) (+ n 8)) (+ n 8))
                      (resume 0 (+ level v))))
                 (drain () level
                  (if (< level 5)
                      (resume 0 0)
                      (resume (- level 5) (- level 5)))))
                (let ((a (K.arrive 6)))
                  (let ((b (K.arrive 7)))
                    (let ((c (K.drain)))
                      (let ((d (K.arrive 9)))
                        (let ((e (K.drain)))
                          (let ((f (K.drain)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 8001207 Int64))
  (call   main (: 0 Int64)) (output (: 503040300 Int64)))
