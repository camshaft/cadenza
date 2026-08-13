(case "rsp1 a ROLE-INVERTING parity splitter — feed routes each value to the even or odd accumulator, flip INVERTS the routing so post-flip evens land in the odd bucket, the flip answers the packed snapshot"
  (input  (do
            (effect S
              (op feed (-> Int64 Int64))
              (op flip (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 0 0)
                ((feed (v) st
                  (match st
                    ((tuple ev od f)
                      (if (= (= (% v 2) 0) (= f 0))
                          (resume (+ ev v) (tuple (+ ev v) od f))
                          (resume (+ od v) (tuple ev (+ od v) f))))))
                 (flip () st
                  (match st
                    ((tuple ev od f)
                      (resume (+ (* 100 ev) od) (tuple ev od (- 1 f)))))))
                (let ((a (S.feed 4)))
                  (let ((b (S.feed n)))
                    (let ((c (S.flip)))
                      (let ((d (S.feed 6)))
                        (let ((e (S.feed 3)))
                          (+ (* 10000 (+ (* 10000 (+ (* 100 a) b)) c)) (+ (* 100 d) e)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 40304030907 Int64))
  (call   main (: 8 Int64)) (output (: 41212000615 Int64)))
