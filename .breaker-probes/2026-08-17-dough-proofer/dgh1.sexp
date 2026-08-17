(case "dgh1 a DOUGH proofer with knock-backs — each proof grows the volume by the time TIMES the gluten strength over two, past twelve the dough is KNOCKED BACK to a THIRD of its risen size while the gluten STRENGTHENS by one (seven-hundred row with the knock count and deflated volume's low digit), the read packs volume gluten and knocks, and the seed's starting gluten knocks back the strong dough mid-run while the weak one rises clean to the brink with every growth compounding the divergence"
  (input  (do
            (effect D
              (op proof (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle D (tuple (: 4 Int64) (+ (: 2 Int64) (% n 3)) (: 0 Int64))
                ((proof (t) st
                  (match st
                    ((tuple vol glu kn)
                      (if (> (+ vol (/ (* t glu) 2)) 12)
                          (resume (+ (: 700 Int64)
                                     (+ (* (+ kn 1) 10)
                                        (% (/ (+ vol (/ (* t glu) 2)) 3) 10)))
                                  (tuple (/ (+ vol (/ (* t glu) 2)) 3) (+ glu 1) (+ kn 1)))
                          (resume (+ (* (/ (* t glu) 2) 10) (% (+ vol (/ (* t glu) 2)) 10))
                                  (tuple (+ vol (/ (* t glu) 2)) glu kn))))))
                 (read () st
                  (match st
                    ((tuple vol glu kn)
                      (resume (+ (* vol 100) (+ (* glu 10) kn)) st)))))
                (let ((a (D.proof (: 3 Int64))))
                  (let ((b (D.proof (: 4 Int64))))
                    (let ((c (D.proof (: 3 Int64))))
                      (let ((f (D.read)))
                        (+ (* 10000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 487140601041 Int64))
  (call   main (: 0 Int64)) (output (: 370417140431 Int64)))
