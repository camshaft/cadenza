(case "glb1 a GLASSBLOWER'S bench — each blow thins the wall by a third of the gather FLOORED at one and halves the gather (a would-be wall under two CRACKS instead, both fields frozen), heating adds gather, the read packs wall gather and cracks, and the seed's opening gather thins fast toward the crack line on one bench while the other's gentle thirds leave wall to spare"
  (input  (do
            (effect G
              (op heat (-> Int64 Int64))
              (op blow (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle G (tuple (+ (: 3 Int64) (* (% n 3) 6)) (: 9 Int64) (: 0 Int64))
                ((heat (g) st
                  (match st
                    ((tuple gather wall cr)
                      (resume (+ (* (+ gather g) 10) (% g 10)) (tuple (+ gather g) wall cr)))))
                 (blow () st
                  (match st
                    ((tuple gather wall cr)
                      (if (< (/ gather 3) 1)
                          (if (< (- wall 1) 2)
                              (resume (+ (: 900 Int64) (+ cr 1)) (tuple gather wall (+ cr 1)))
                              (resume (+ (: 10 Int64) (% (- wall 1) 10))
                                      (tuple (/ gather 2) (- wall 1) cr)))
                          (if (< (- wall (/ gather 3)) 2)
                              (resume (+ (: 900 Int64) (+ cr 1)) (tuple gather wall (+ cr 1)))
                              (resume (+ (* (/ gather 3) 10) (% (- wall (/ gather 3)) 10))
                                      (tuple (/ gather 2) (- wall (/ gather 3)) cr)))))))
                 (read () st
                  (match st
                    ((tuple gather wall cr)
                      (resume (+ (* wall 100) (+ (* gather 10) cr)) st)))))
                (let ((a (G.blow)))
                  (let ((b (G.heat (: 4 Int64))))
                    (let ((c (G.blow)))
                      (let ((d (G.blow)))
                        (let ((f (G.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 360840240130320 Int64))
  (call   main (: 0 Int64)) (output (: 180540170160610 Int64)))
