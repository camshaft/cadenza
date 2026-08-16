(case "crs1 a CAROUSEL of four gondolas — board fills the gondola at the gate only if its mask bit is empty (the refused rider walks away) and the wheel rotates either way, unload clears a named gondola's bit answering distinct hit and miss codes without rotating, count packs riders and position, and the seed's starting rotation decides WHICH gondola meets each boarding attempt"
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
                      (let ((d (W.board)))
                        (let ((e (W.board)))
                          (let ((f (W.unload (: 1 Int64))))
                            (let ((g (W.count)))
                              (+ (* 100 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2101110200103110122 Int64))
  (call   main (: 0 Int64)) (output (: 103190202101110130 Int64)))
