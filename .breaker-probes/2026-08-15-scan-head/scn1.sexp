(case "scn1 a DISK-HEAD seek tracker — req answers the absolute seek cost, moves the head, and counts DIRECTION FLIPS (a zero-cost repeat request flips nothing), stat packs direction and flip count, and the seed's starting head position changes which requests reverse"
  (input  (do
            (effect S
              (op req (-> Int64 Int64))
              (op stat (-> Int64)))
            (def (iabs (: v Int64)) (if (< v 0) (- 0 v) v))
            (def (main (: n Int64))
              (handle S (tuple n (: 1 Int64) (: 0 Int64))
                ((req (t) st
                  (match st
                    ((tuple head d flips)
                      (if (= t head)
                          (resume 0 st)
                          (if (< t head)
                              (if (= d -1)
                                  (resume (iabs (- t head)) (tuple t -1 flips))
                                  (resume (iabs (- t head)) (tuple t -1 (+ flips 1))))
                              (if (= d 1)
                                  (resume (iabs (- t head)) (tuple t 1 flips))
                                  (resume (iabs (- t head)) (tuple t 1 (+ flips 1)))))))))
                 (stat () st
                  (match st
                    ((tuple head d flips) (resume (+ (* d 100) flips) st)))))
                (let ((a (S.req 6)))
                  (let ((b (S.req 14)))
                    (let ((c (S.req 3)))
                      (let ((e (S.req 3)))
                        (let ((f (S.req 9)))
                          (let ((g (S.stat)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) e)) f)) g)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 40811000704 Int64))
  (call   main (: 0 Int64)) (output (: 60811000702 Int64)))
