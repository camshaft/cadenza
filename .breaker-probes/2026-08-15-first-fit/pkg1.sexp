(case "pkg1 FIRST-FIT bin packing over two capacity-ten bins — place walks the bins in order answering bin-index-times-a-hundred plus the remaining room (or -1 when nothing fits), loads packs both levels, and the seed's first item cascades every later placement"
  (input  (do
            (effect P
              (op place (-> Int64 Int64))
              (op loads (-> Int64)))
            (def (main (: n Int64))
              (handle P (tuple (: 0 Int64) (: 0 Int64))
                ((place (w) st
                  (match st
                    ((tuple b0 b1)
                      (if (< (+ b0 w) 11)
                          (resume (- 10 (+ b0 w)) (tuple (+ b0 w) b1))
                          (if (< (+ b1 w) 11)
                              (resume (+ 100 (- 10 (+ b1 w))) (tuple b0 (+ b1 w)))
                              (resume -1 st))))))
                 (loads () st
                  (match st
                    ((tuple b0 b1) (resume (+ (* b0 10) b1) st)))))
                (let ((a (P.place (+ (% n 4) 3))))
                  (let ((b (P.place 6)))
                    (let ((c (P.place 5)))
                      (let ((d (P.place 8)))
                        (let ((e (P.place 4)))
                          (let ((f (P.loads)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5103999999100110 Int64))
  (call   main (: 0 Int64)) (output (: 7001104999101099 Int64)))