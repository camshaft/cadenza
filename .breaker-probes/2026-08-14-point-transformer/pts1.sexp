(case "pts1 a 2D POINT under quarter-turn rotations and translations — rot maps (x,y) to (y,-x) answering a sign-tagged composite, mv translates answering the Manhattan norm, quad reads the quadrant, and the seeds trace different orbits around the origin"
  (input  (do
            (effect P
              (op rot (-> Int64))
              (op mv (-> Int64 Int64 Int64))
              (op quad (-> Int64)))
            (def (iabs (: v Int64)) (if (< v 0) (- 0 v) v))
            (def (main (: n Int64))
              (handle P (tuple n (: 3 Int64))
                ((rot () st
                  (match st
                    ((tuple x y)
                      (resume (+ (* y 10) (if (< (- 0 x) 0) -1 1)) (tuple y (- 0 x))))))
                 (mv (dx dy) st
                  (match st
                    ((tuple x y)
                      (resume (+ (iabs (+ x dx)) (iabs (+ y dy))) (tuple (+ x dx) (+ y dy))))))
                 (quad () st
                  (match st
                    ((tuple x y)
                      (if (< x 0)
                          (if (< y 0) (resume 3 st) (resume 2 st))
                          (if (< y 0) (resume 4 st) (resume 1 st)))))))
                (let ((a (P.rot)))
                  (let ((b (P.mv 5 -2)))
                    (let ((c (P.rot)))
                      (let ((d (P.quad)))
                        (let ((e (P.mv -1 8)))
                          (let ((f (P.rot)))
                            (let ((g (P.quad)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 29187903130101 Int64))
  (call   main (: 0 Int64)) (output (: 31097903030101 Int64)))
