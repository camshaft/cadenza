(case "btr3 the digitizer at FOUR steps (ladder probe) — acc still mid-reconstruction (negative on the ten seed)"
  (input  (do
            (effect T
              (op step (-> Int64))
              (op check (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (+ (: 40 Int64) n) (: 0 Int64) (: 1 Int64))
                ((step () st
                  (match st
                    ((tuple v acc w)
                      (let ((r (% v 3)))
                        (if (= r 2)
                            (resume (: 9 Int64) (tuple (/ (+ v 1) 3) (- acc w) (* w 3)))
                            (if (= r 1)
                                (resume (: 1 Int64) (tuple (/ (- v 1) 3) (+ acc w) (* w 3)))
                                (resume (: 0 Int64) (tuple (/ v 3) acc (* w 3)))))))))
                 (check () st
                  (match st ((tuple v acc w) (resume acc st)))))
                (let ((a (T.step)))
                  (let ((b (T.step)))
                    (let ((c (T.step)))
                      (let ((d (T.step)))
                        (let ((f (T.check)))
                          (+ (* 100 (+ (* 10 (+ (* 10 (+ (* 10 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 990869 Int64))
  (call   main (: 0 Int64)) (output (: 111140 Int64)))
