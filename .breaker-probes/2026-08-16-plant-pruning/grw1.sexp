(case "grw1 PLANT growth with pruning stress — day grows by the current rate, prune cuts back to the target answering the clippings AND slows the rate by one (bottoming at one) but only when it actually cut, and the fast grower gets pruned TWICE (stressed twice) while the slow one skips the first prune entirely as a zero-clip row"
  (input  (do
            (effect G
              (op day (-> Int64))
              (op prune (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G (tuple (: 0 Int64) (+ (% n 4) 2))
                ((day () st
                  (match st
                    ((tuple height rate)
                      (resume (+ height rate) (tuple (+ height rate) rate)))))
                 (prune (h) st
                  (match st
                    ((tuple height rate)
                      (if (< h height)
                          (if (< 1 rate)
                              (resume (- height h) (tuple h (- rate 1)))
                              (resume (- height h) (tuple h 1)))
                          (resume 0 st))))))
                (let ((a (G.day)))
                  (let ((b (G.day)))
                    (let ((c (G.prune 5)))
                      (let ((d (G.day)))
                        (let ((e (G.day)))
                          (let ((f (G.prune 5)))
                            (let ((g (G.day)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 4080308110607 Int64))
  (call   main (: 0 Int64)) (output (: 2040006080306 Int64)))
