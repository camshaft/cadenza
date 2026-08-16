(case "tpwJ minimal no-local-slot repro — a let-bound NESTED-IF whose OUTER condition reads a state field and whose result feeds >=3 consumers, at >=3 dispatches with a state-reading second op"
  (input  (do
            (effect K (op type (-> Int64 Int64)) (op fin (-> Int64)))
            (def (main (: n Int64))
              (handle K (tuple (: 0 Int64) (: 0 Int64))
                ((type (x) st
                  (match st
                    ((tuple col k)
                      (let ((c2 (+ (if (= (% (+ k 1) 3) 0)
                                       (if (= (% n 3) 0) col (* (+ (/ col 4) 1) 4))
                                       col)
                                   x)))
                        (if (>= c2 8)
                            (resume (- c2 8) (tuple (- c2 8) (+ k 1)))
                            (resume c2 (tuple c2 (+ k 1))))))))
                 (fin () st
                  (match st ((tuple col k) (resume (+ (* col 10) k) st)))))
                (let ((a (K.type (: 3 Int64))))
                  (let ((b (K.type (: 5 Int64))))
                    (let ((c (K.type (: 2 Int64))))
                      (let ((f (K.fin)))
                        (+ (* 1000 (+ (* 100 (+ (* 10 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 3006063 Int64))
  (call   main (: 0 Int64)) (output (: 3002023 Int64)))
