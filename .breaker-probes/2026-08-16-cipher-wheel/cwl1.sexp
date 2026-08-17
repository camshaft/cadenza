(case "cwl1 a CIPHER WHEEL that slips every third letter — encoding adds the offset mod twenty-six and every THIRD encode advances the wheel one notch (a nine-tagged row counting the slip), other rows carry the stroke's residue tag, the read packs offset strokes and slips, and the seed's starting offset wraps one alphabet where the other doesn't so the ciphertext rows share nothing"
  (input  (do
            (effect C
              (op enc (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle C (tuple (+ (: 3 Int64) (* (% n 3) 7)) (: 0 Int64) (: 0 Int64))
                ((enc (c) st
                  (match st
                    ((tuple off k sl)
                      (if (= (% (+ k 1) 3) 0)
                          (resume (+ (* (% (+ c off) 26) 10) 9)
                                  (tuple (+ off 1) (+ k 1) (+ sl 1)))
                          (resume (+ (* (% (+ c off) 26) 10) (% (+ k 1) 3))
                                  (tuple off (+ k 1) sl))))))
                 (read () st
                  (match st
                    ((tuple off k sl)
                      (resume (+ (* off 100) (+ (* k 10) sl)) st)))))
                (let ((a (C.enc (: 7 Int64))))
                  (let ((b (C.enc (: 20 Int64))))
                    (let ((c (C.enc (: 4 Int64))))
                      (let ((d (C.enc (: 25 Int64))))
                        (let ((f (C.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1710421491011141 Int64))
  (call   main (: 0 Int64)) (output (: 1012320790310441 Int64)))
