(case "kln1 a KILN firing schedule with crack risk — a heat gains temperature and a ramp over fifteen degrees CRACKS the ware (counted, answering nine hundred plus the temperature's low digits) where a gentle ramp answers the stage from integer division with the within-stage remainder, a soak at an exact stage boundary earns a quality point (else echoing the offset), the read packs all three, and the seed's first ramp is gentle on one run and cracking on the other so stage arithmetic and quality diverge"
  (input  (do
            (effect K
              (op heat (-> Int64 Int64))
              (op soak (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle K (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((heat (d) st
                  (match st
                    ((tuple t q c)
                      (if (> d 15)
                          (resume (+ (: 900 Int64) (% (+ t d) 100)) (tuple (+ t d) q (+ c 1)))
                          (resume (+ (* (/ (+ t d) 25) 100) (% (+ t d) 25)) (tuple (+ t d) q c))))))
                 (soak () st
                  (match st
                    ((tuple t q c)
                      (if (= (% t 25) 0)
                          (resume (+ (* (+ q 1) 10) 1) (tuple t (+ q 1) c))
                          (resume (* (% t 25) 10) st)))))
                 (read () st
                  (match st
                    ((tuple t q c)
                      (resume (+ (* t 100) (+ (* q 10) c)) st)))))
                (let ((a (K.heat (+ (: 10 Int64) (* (% n 3) 8)))))
                  (let ((b (K.soak)))
                    (let ((c (K.heat (: 15 Int64))))
                      (let ((d (K.soak)))
                        (let ((f (K.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 9181801080803301 Int64))
  (call   main (: 0 Int64)) (output (: 101001000112510 Int64)))
