(case "cmbB the cmb1 arm with the coefficient compound HOISTED through a match binder"
  (input  (do
            (effect W
              (op next (-> Int64))
              (op apex (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (: 1 Int64) (: 0 Int64) (: 1 Int64))
                ((next () st
                  (match st
                    ((tuple c k mx)
                      (if (< (+ 6 (% n 4)) (+ k 1))
                          (resume -1 st)
                          (match (/ (* c (- (+ 6 (% n 4)) k)) (+ k 1))
                            (c2
                              (if (< mx c2)
                                  (resume c2 (tuple c2 (+ k 1) c2))
                                  (resume c2 (tuple c2 (+ k 1) mx)))))))))
                 (apex () st
                  (match st ((tuple c k mx) (resume mx st)))))
                (let ((a (W.next)))
                  (let ((b (W.next)))
                    (let ((c (W.next)))
                      (let ((d (W.next)))
                        (let ((e (W.next)))
                          (let ((f (W.next)))
                            (let ((g (W.next)))
                              (let ((h (W.apex)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 828567056280870 Int64))
  (call   main (: 0 Int64)) (output (: 615201506009920 Int64)))