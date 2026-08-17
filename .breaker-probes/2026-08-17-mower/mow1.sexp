(case "mow1 a MOWER with a grass-catcher — each pass cuts the LESSER of the standing grass and five moving it to the catcher, a catcher past eight AUTO-EMPTIES (counted, a seven-hundred row with the empty count and the cut's low digit), otherwise the answer packs the cut with the catcher level, growth adds grass, the read packs grass catcher and empties, and the seed's lawn auto-empties on the SECOND pass for one keeper and the THIRD for the other"
  (input  (do
            (effect G
              (op grow (-> Int64 Int64))
              (op mow (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle G (tuple (+ (: 3 Int64) (* (% n 3) 4)) (: 0 Int64) (: 0 Int64))
                ((grow (g) st
                  (match st
                    ((tuple grass cat emp)
                      (resume (+ (* (+ grass g) 10) (% g 10)) (tuple (+ grass g) cat emp)))))
                 (mow () st
                  (match st
                    ((tuple grass cat emp)
                      (if (< grass 5)
                          (if (> (+ cat grass) 8)
                              (resume (+ (: 700 Int64) (+ (* (+ emp 1) 10) (% grass 10)))
                                      (tuple (: 0 Int64) (: 0 Int64) (+ emp 1)))
                              (resume (+ (* grass 10) (% (+ cat grass) 10))
                                      (tuple (: 0 Int64) (+ cat grass) emp)))
                          (if (> (+ cat 5) 8)
                              (resume (+ (: 700 Int64) (+ (* (+ emp 1) 10) 5))
                                      (tuple (- grass 5) (: 0 Int64) (+ emp 1)))
                              (resume (+ (: 50 Int64) (% (+ cat 5) 10))
                                      (tuple (- grass 5) (+ cat 5) emp)))))))
                 (read () st
                  (match st
                    ((tuple grass cat emp)
                      (resume (+ (* grass 100) (+ (* cat 10) emp)) st)))))
                (let ((a (G.mow)))
                  (let ((b (G.grow (: 6 Int64))))
                    (let ((c (G.mow)))
                      (let ((d (G.mow)))
                        (let ((f (G.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 550867150330031 Int64))
  (call   main (: 0 Int64)) (output (: 330660587110001 Int64)))
