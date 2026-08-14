(case "plt1 a LONGEST-PLATEAU tracker — equality runs extend or reset, and when a run overtakes the best BOTH the length and the plateau VALUE update; the tie does not steal the crown"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple -999 0 0 -1)
                ((feed (v) st
                  (match st
                    ((tuple prev run bl bv)
                      (let ((r2 (if (= v prev) (+ run 1) 1)))
                        (let ((bl2 (if (> r2 bl) r2 bl)))
                          (let ((bv2 (if (> r2 bl) v bv)))
                            (resume (+ (* bl2 10) (% bv2 10))
                                    (tuple v r2 bl2 bv2)))))))))
                (let ((a (S.feed 4)))
                  (let ((b (S.feed 4)))
                    (let ((c (S.feed n)))
                      (let ((d (S.feed n)))
                        (let ((e (S.feed n)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 1424344454 Int64))
  (call   main (: 7 Int64)) (output (: 1424242437 Int64)))
