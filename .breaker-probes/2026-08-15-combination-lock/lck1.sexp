(case "lck1 a COMBINATION-LOCK matcher — press advances the cursor against the three-digit secret whose MIDDLE digit is seed-shaped, a wrong press falls back to one if it re-matches the first digit else to zero (answering the negated or zero fallback), and open pays a hundred-plus-count on a full match; the SAME eight presses open the lock in the FIRST half on one seed and the SECOND half on the other"
  (input  (do
            (effect L
              (op press (-> Int64 Int64))
              (op opn (-> Int64)))
            (def (digit-at (: n Int64) (: i Int64))
              (if (= i 0) 3 (if (= i 1) (+ (% n 4) 1) 7)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((press (d) st
                  (match st
                    ((tuple cur opens)
                      (if (= d (digit-at n cur))
                          (resume (+ cur 1) (tuple (+ cur 1) opens))
                          (if (= d 3)
                              (resume -1 (tuple 1 opens))
                              (resume 0 (tuple 0 opens)))))))
                 (opn () st
                  (match st
                    ((tuple cur opens)
                      (if (= cur 3)
                          (resume (+ 101 opens) (tuple 0 (+ opens 1)))
                          (resume (- 0 cur) st))))))
                (let ((a (L.press 3)))
                  (let ((b (L.press 3)))
                    (let ((c (L.press 7)))
                      (let ((d (L.opn)))
                        (let ((e (L.press 3)))
                          (let ((f (L.press 1)))
                            (let ((g (L.press 7)))
                              (let ((h (L.opn)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 102040101000000 Int64))
  (call   main (: 0 Int64)) (output (: 99000001020401 Int64)))
