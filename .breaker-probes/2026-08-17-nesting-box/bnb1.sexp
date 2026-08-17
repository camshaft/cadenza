(case "bnb1 a NESTING BOX season — hatching turns the CEILING-HALF of the eggs to chicks (the empty clutch answering nine hundred), fledging sends all but ONE chick out (the last stays as next year's sitter, an eight-hundred row when the brood is too small), laying adds eggs, the read packs eggs chicks and fledged, and the seed's clutch ceil-halves to different splits every hatch so the fledge counts diverge"
  (input  (do
            (effect N
              (op lay (-> Int64 Int64))
              (op hatch (-> Int64))
              (op fledge (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle N (tuple (+ (: 1 Int64) (* (% n 3) 2)) (: 0 Int64) (: 0 Int64))
                ((lay (k) st
                  (match st
                    ((tuple eggs ch fl)
                      (resume (+ (* (+ eggs k) 10) (% k 10)) (tuple (+ eggs k) ch fl)))))
                 (hatch () st
                  (match st
                    ((tuple eggs ch fl)
                      (if (= eggs 0)
                          (resume (: 900 Int64) st)
                          (resume (+ (* (/ (+ eggs 1) 2) 10) (% (- eggs (/ (+ eggs 1) 2)) 10))
                                  (tuple (- eggs (/ (+ eggs 1) 2))
                                         (+ ch (/ (+ eggs 1) 2))
                                         fl))))))
                 (fledge () st
                  (match st
                    ((tuple eggs ch fl)
                      (if (>= ch 2)
                          (resume (+ (* (+ fl (- ch 1)) 10) 1)
                                  (tuple eggs (: 1 Int64) (+ fl (- ch 1))))
                          (resume (+ (: 800 Int64) ch) st)))))
                 (read () st
                  (match st
                    ((tuple eggs ch fl)
                      (resume (+ (* eggs 100) (+ (* ch 10) fl)) st)))))
                (let ((a (N.hatch)))
                  (let ((b (N.lay (: 3 Int64))))
                    (let ((c (N.hatch)))
                      (let ((d (N.fledge)))
                        (let ((f (N.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 210430220310213 Int64))
  (call   main (: 0 Int64)) (output (: 100330210210112 Int64)))
