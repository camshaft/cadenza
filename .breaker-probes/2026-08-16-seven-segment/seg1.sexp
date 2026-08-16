(case "seg1 a SEVEN-SEGMENT display driver — show decodes a digit to its segment mask through a nested-if ladder, answers the lit-segment popcount packed with how many segments FLIPPED against the previous mask (an XOR popcount through a recursive helper), stats totals both and reads the live mask, and the seed picks the middle digit so the flip counts diverge while the first row agrees"
  (input  (do
            (effect G
              (op show (-> Int64 Int64))
              (op stats (-> Int64)))
            (def (pop (: b Int64) (: acc Int64))
              (if (= b 0) acc (pop (>> b 1) (+ acc (& b 1)))))
            (def (segmask (: d Int64))
              (if (= d 0) 63
                (if (= d 1) 6
                  (if (= d 2) 91
                    (if (= d 3) 79 102)))))
            (def (main (: n Int64))
              (handle G (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((show (d) st
                  (match st
                    ((tuple mask lit chg)
                      (resume (+ (* (pop (segmask d) 0) 10) (pop (^ (segmask d) mask) 0))
                              (tuple (segmask d)
                                     (+ lit (pop (segmask d) 0))
                                     (+ chg (pop (^ (segmask d) mask) 0)))))))
                 (stats () st
                  (match st
                    ((tuple mask lit chg)
                      (resume (+ (* lit 100) (+ (* chg 10) (pop mask 0))) st)))))
                (let ((a (G.show (: 1 Int64))))
                  (let ((b (G.show (% (+ 2 (% n 3)) 5))))
                    (let ((c (G.show (: 4 Int64))))
                      (let ((f (G.stats)))
                        (+ (* 10000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 220530431184 Int64))
  (call   main (: 0 Int64)) (output (: 220550451224 Int64)))
