(case "bwl1 a SPARE-CHAIN scorer — each roll adds its pins plus a DOUBLE when the previous two rolls summed to ten, threading a two-roll history through the state, and only the seed whose opening pair hits ten fires the bonus so the scores drift apart from the third row"
  (input  (do
            (effect B
              (op roll (-> Int64 Int64))
              (op total (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (: -1 Int64) (: -1 Int64) (: 0 Int64))
                ((roll (p) st
                  (match st
                    ((tuple p2 p1 score)
                      (if (< p2 0)
                          (resume (+ score p) (tuple p1 p (+ score p)))
                          (if (= (+ p2 p1) 10)
                              (resume (+ score (* p 2)) (tuple p1 p (+ score (* p 2))))
                              (resume (+ score p) (tuple p1 p (+ score p))))))))
                 (total () st
                  (match st ((tuple p2 p1 score) (resume score st)))))
                (let ((a (B.roll (+ (% n 6) 3))))
                  (let ((b (B.roll 3)))
                    (let ((c (B.roll 5)))
                      (let ((d (B.roll 6)))
                        (let ((e (B.roll 4)))
                          (let ((f (B.total)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 71020263030 Int64))
  (call   main (: 0 Int64)) (output (: 30611172121 Int64)))
