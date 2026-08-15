(case "bch1 a BATCH-coalescing writer — write buffers answering the count until the THIRD item self-flushes answering a hundred plus the batch sum, sync force-flushes the partial answering its sum or -1 when empty, and two seed-shaped values land in DIFFERENT batches so both the partial-sync and the self-flush rows carry the seed"
  (input  (do
            (effect W
              (op write (-> Int64 Int64))
              (op sync (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (: 0 Int64) (: 0 Int64))
                ((write (v) st
                  (match st
                    ((tuple bsum bn)
                      (if (= (+ bn 1) 3)
                          (resume (+ 100 (+ bsum v)) (tuple 0 0))
                          (resume (+ bn 1) (tuple (+ bsum v) (+ bn 1)))))))
                 (sync () st
                  (match st
                    ((tuple bsum bn)
                      (if (= bn 0)
                          (resume -1 st)
                          (resume bsum (tuple 0 0)))))))
                (let ((a (W.write (+ (% n 4) 2))))
                  (let ((b (W.write 4)))
                    (let ((c (W.sync)))
                      (let ((d (W.write 6)))
                        (let ((e (W.write (+ (% n 3) 1))))
                          (let ((f (W.write 8)))
                            (let ((g (W.sync)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1020801031599 Int64))
  (call   main (: 0 Int64)) (output (: 1020601031499 Int64)))
