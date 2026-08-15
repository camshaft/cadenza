(case "rpc1 a RIPPLE-CARRY counter — inc answers the number of bits FLIPPED by that increment (the XOR of old and new popcounted, the amortized-analysis witness), pop reads the live bit count, and the seeds start at 10 and 0 so the carry chains fire at different steps"
  (input  (do
            (effect C
              (op inc (-> Int64))
              (op pop (-> Int64)))
            (def (bits (: b Int64) (: acc Int64))
              (if (= b 0) acc (bits (>> b 1) (+ acc (& b 1)))))
            (def (main (: n Int64))
              (handle C (: n Int64)
                ((inc () v
                  (resume (bits (^ v (+ v 1)) 0) (+ v 1)))
                 (pop () v (resume (bits v 0) v)))
                (let ((a (C.inc)))
                  (let ((b (C.inc)))
                    (let ((c (C.pop)))
                      (let ((d (C.inc)))
                        (let ((e (C.inc)))
                          (let ((f (C.pop)))
                            (let ((g (C.inc)))
                              (let ((h (C.pop)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 103020102030104 Int64))
  (call   main (: 0 Int64)) (output (: 102010103010102 Int64)))
