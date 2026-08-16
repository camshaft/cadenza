(case "tid1 a TIDAL predictor on a triangle wave — read advances one hour answering the level (rising to the seed amplitude then falling), moor answers one when the current level covers the draft or the negated shortfall, and the smaller amplitude peaks EARLIER so the same moor probes catch one tide still rising and the other already ebbing"
  (input  (do
            (effect T
              (op read (-> Int64))
              (op moor (-> Int64 Int64)))
            (def (levl (: t Int64) (: amp Int64))
              (if (< amp (% t (* 2 amp)))
                  (- (* 2 amp) (% t (* 2 amp)))
                  (% t (* 2 amp))))
            (def (main (: n Int64))
              (handle T (: 0 Int64)
                ((read () t
                  (resume (levl (+ t 1) (+ (% n 3) 2)) (+ t 1)))
                 (moor (draft) t
                  (if (< (levl t (+ (% n 3) 2)) draft)
                      (resume (- (levl t (+ (% n 3) 2)) draft) t)
                      (resume 1 t))))
                (let ((a (T.read)))
                  (let ((b (T.read)))
                    (let ((c (T.moor 2)))
                      (let ((d (T.read)))
                        (let ((e (T.read)))
                          (let ((f (T.moor 3)))
                            (let ((g (T.read)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1020103019901 Int64))
  (call   main (: 0 Int64)) (output (: 1020100999701 Int64)))
