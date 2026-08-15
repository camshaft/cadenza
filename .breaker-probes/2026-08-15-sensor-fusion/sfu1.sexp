(case "sfu1 TWO-SENSOR weighted fusion — each read folds the sample into a running weighted mean by est*wsum plus v*w over the grown weight (truncating), the trusted sensor carries weight three while the seed shapes the second sensor's trust, and the final draw reads the accumulated weight"
  (input  (do
            (effect F
              (op read1 (-> Int64 Int64))
              (op read2 (-> Int64 Int64))
              (op wtot (-> Int64)))
            (def (main (: n Int64))
              (handle F (tuple (: 0 Int64) (: 0 Int64))
                ((read1 (v) st
                  (match st
                    ((tuple est wsum)
                      (resume (/ (+ (* est wsum) (* v 3)) (+ wsum 3))
                              (tuple (/ (+ (* est wsum) (* v 3)) (+ wsum 3)) (+ wsum 3))))))
                 (read2 (v) st
                  (match st
                    ((tuple est wsum)
                      (resume (/ (+ (* est wsum) (* v (+ (% n 4) 1))) (+ wsum (+ (% n 4) 1)))
                              (tuple (/ (+ (* est wsum) (* v (+ (% n 4) 1))) (+ wsum (+ (% n 4) 1))) (+ wsum (+ (% n 4) 1)))))))
                 (wtot () st
                  (match st ((tuple est wsum) (resume wsum st)))))
                (let ((a (F.read1 12)))
                  (let ((b (F.read2 20)))
                    (let ((c (F.read1 6)))
                      (let ((d (F.read2 16)))
                        (let ((e (F.wtot)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1216121312 Int64))
  (call   main (: 0 Int64)) (output (: 1214101008 Int64)))
