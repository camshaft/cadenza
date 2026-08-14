(case "lap1 a STOPWATCH with lap splits — tick advances the clock by a seed stride, lap answers the split since the last mark and remembers it, best tracks the MINIMUM split (seeded -1 sentinel replaced on first lap), and the middle lap of three is the unique best on both seeds"
  (input  (do
            (effect W
              (op tick (-> Int64))
              (op lap (-> Int64))
              (op bst (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (: 0 Int64) (: 0 Int64) (: -1 Int64))
                ((tick () st
                  (match st
                    ((tuple t last best)
                      (resume (+ t (+ (% n 3) 2)) (tuple (+ t (+ (% n 3) 2)) last best)))))
                 (lap () st
                  (match st
                    ((tuple t last best)
                      (if (< best 0)
                          (resume (- t last) (tuple t t (- t last)))
                          (if (< (- t last) best)
                              (resume (- t last) (tuple t t (- t last)))
                              (resume (- t last) (tuple t t best)))))))
                 (bst () st
                  (match st ((tuple t last best) (resume best st)))))
                (let ((a (W.tick)))
                  (let ((b (W.tick)))
                    (let ((c (W.lap)))
                      (let ((d (W.tick)))
                        (let ((e (W.lap)))
                          (let ((f (W.tick)))
                            (let ((g (W.tick)))
                              (let ((h (W.lap)))
                                (let ((i (W.bst)))
                                  (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)) i))))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 30606090312150603 Int64))
  (call   main (: 0 Int64)) (output (: 20404060208100402 Int64)))
