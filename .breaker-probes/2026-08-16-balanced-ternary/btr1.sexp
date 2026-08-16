(case "btr1 a BALANCED-TERNARY digitizer — each step peels the low signed trit of the seeded value (remainder two means digit MINUS-ONE displayed as nine with a carry, remainder one means plus-one, zero passes through), a running weighted accumulator resummes the trits, and the closing check answers the accumulator which must RECONSTRUCT the original value after transiting negative partial sums on the ten seed"
  (input  (do
            (effect T
              (op step (-> Int64))
              (op check (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (+ (: 40 Int64) n) (: 0 Int64) (: 1 Int64))
                ((step () st
                  (match st
                    ((tuple v acc w)
                      (if (= (% v 3) 2)
                          (resume (: 9 Int64) (tuple (/ (+ v 1) 3) (- acc w) (* w 3)))
                          (if (= (% v 3) 1)
                              (resume (: 1 Int64) (tuple (/ (- v 1) 3) (+ acc w) (* w 3)))
                              (resume (: 0 Int64) (tuple (/ v 3) acc (* w 3))))))))
                 (check () st
                  (match st ((tuple v acc w) (resume acc st)))))
                (let ((a (T.step)))
                  (let ((b (T.step)))
                    (let ((c (T.step)))
                      (let ((d (T.step)))
                        (let ((e (T.step)))
                          (let ((f (T.check)))
                            (+ (* 100 (+ (* 10 (+ (* 10 (+ (* 10 (+ (* 10 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 9909150 Int64))
  (call   main (: 0 Int64)) (output (: 1111040 Int64)))
