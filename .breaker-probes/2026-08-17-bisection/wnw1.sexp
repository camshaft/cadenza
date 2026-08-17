(case "wnw1 a BISECTION searcher whose starting window is picked by a MATCH IN THE HANDLE'S INIT POSITION — the init destructures a seed tuple and an if ladder inside the match arm chooses among three windows, narrow halves the window toward the guess counting probes and answering side midpoint and count, a CLOSED window answers a frozen readout without probing, span reads the gap, and the tight seed window closes two probes early so its last narrows freeze while the wide window is still hunting"
  (input  (do
            (effect L
              (op narrow (-> Int64 Int64))
              (op span (-> Int64)))
            (def (main (: n Int64))
              (handle L (match (tuple (% n 3) (: 0 Int64))
                          ((tuple r z)
                            (if (= r 0) (tuple (: 2 Int64) (: 9 Int64) z)
                                (if (= r 1) (tuple (: 4 Int64) (: 6 Int64) z)
                                    (tuple (: 1 Int64) (: 12 Int64) z)))))
                ((narrow (g) st
                  (match st
                    ((tuple lo hi probes)
                      (if (>= lo hi)
                          (resume (+ (: 300 Int64) (+ (* (% lo 10) 10) (% probes 10))) st)
                          (let ((mid (/ (+ lo hi) 2)))
                            (if (<= g mid)
                                (resume (+ (: 100 Int64) (+ (* (% mid 10) 10) (% (+ probes 1) 10)))
                                        (tuple lo mid (+ probes 1)))
                                (resume (+ (: 200 Int64) (+ (* (% mid 10) 10) (% (+ probes 1) 10)))
                                        (tuple (+ mid 1) hi (+ probes 1)))))))))
                 (span () st
                  (match st
                    ((tuple lo hi probes)
                      (resume (+ (* (- hi lo) 10) (% probes 10)) st)))))
                (let ((a (L.narrow 5)))
                  (let ((b (L.narrow 3)))
                    (let ((c (L.span)))
                      (let ((d (L.narrow 7)))
                        (let ((e (L.span)))
                          (let ((f (L.narrow 2)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 151142002342002342 Int64))
  (call   main (: 0 Int64)) (output (: 151132012223003333 Int64)))
