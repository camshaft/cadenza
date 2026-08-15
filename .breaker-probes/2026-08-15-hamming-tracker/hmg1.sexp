(case "hmg1 a HAMMING-distance tracker with an XOR-folding reference — cmp answers the popcount of the value against the reference remembering the value, lock folds the last value INTO the reference by XOR answering the old one, and the seed reference propagates through both locks so the distance rows diverge after the first fold"
  (input  (do
            (effect H
              (op cmp (-> Int64 Int64))
              (op lock (-> Int64)))
            (def (bits (: b Int64) (: acc Int64))
              (if (= b 0) acc (bits (>> b 1) (+ acc (& b 1)))))
            (def (main (: n Int64))
              (handle H (tuple (+ n 5) (: 0 Int64))
                ((cmp (v) st
                  (match st
                    ((tuple ref last) (resume (bits (^ v ref) 0) (tuple ref v)))))
                 (lock () st
                  (match st
                    ((tuple ref last) (resume ref (tuple (^ last ref) last))))))
                (let ((a (H.cmp 7)))
                  (let ((b (H.cmp 12)))
                    (let ((c (H.lock)))
                      (let ((d (H.cmp 7)))
                        (let ((e (H.cmp 15)))
                          (let ((f (H.lock)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10215010203 Int64))
  (call   main (: 0 Int64)) (output (: 10205030209 Int64)))
