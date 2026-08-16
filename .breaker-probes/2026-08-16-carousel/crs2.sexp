(case "crs2 the CAROUSEL at three ops plus count — bitmask boarding at the seed-rotated gate, non-rotating unload with hit and miss codes, one seed's unload HITS the gondola the other seed never filled"
  (input  (do
            (effect W
              (op board (-> Int64))
              (op unload (-> Int64 Int64))
              (op count (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (% n 4) (: 0 Int64) (: 0 Int64))
                ((board () st
                  (match st
                    ((tuple pos mask riders)
                      (if (= (& (>> mask (% (- 4 pos) 4)) 1) 0)
                          (resume (+ (* (% (- 4 pos) 4) 10) 1)
                                  (tuple (% (+ pos 1) 4)
                                         (| mask (<< (: 1 Int64) (% (- 4 pos) 4)))
                                         (+ riders 1)))
                          (resume (* (% (- 4 pos) 4) 10)
                                  (tuple (% (+ pos 1) 4) mask riders))))))
                 (unload (g) st
                  (match st
                    ((tuple pos mask riders)
                      (if (= (& (>> mask g) 1) 1)
                          (resume (+ (: 100 Int64) g)
                                  (tuple pos (^ mask (<< (: 1 Int64) g)) (- riders 1)))
                          (resume (+ (: 900 Int64) g) st)))))
                 (count () st
                  (match st ((tuple pos mask riders) (resume (+ (* riders 10) pos) st)))))
                (let ((a (W.board)))
                  (let ((b (W.board)))
                    (let ((c (W.unload (: 2 Int64))))
                      (let ((g (W.count)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) g)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 21011102010 Int64))
  (call   main (: 0 Int64)) (output (: 1031902022 Int64)))
