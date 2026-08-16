(case "brg1 a DRAWBRIDGE cycling between road and river — cars pass only while CLOSED (blocked answers nine hundred plus the toll count unchanged), boats queue, a cycle CLOSES an open span reporting the fresh queue or OPENS for a waiting fleet sailing every queued boat at once or idles at zero, and the seed's initial fleet makes the first cycle a mass sailing on one run and a NO-OP on the other so the same car meets an open span or a closed one"
  (input  (do
            (effect B
              (op car (-> Int64))
              (op boat (-> Int64))
              (op cycle (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (: 0 Int64) (% n 3) (: 0 Int64))
                ((car () st
                  (match st
                    ((tuple op boats cars)
                      (if (= op 0)
                          (resume (* (+ cars 1) 10) (tuple op boats (+ cars 1)))
                          (resume (+ (: 900 Int64) cars) st)))))
                 (boat () st
                  (match st
                    ((tuple op boats cars)
                      (resume (+ (* (+ boats 1) 10) op) (tuple op (+ boats 1) cars)))))
                 (cycle () st
                  (match st
                    ((tuple op boats cars)
                      (if (= op 1)
                          (resume (+ (: 500 Int64) (* boats 10)) (tuple (: 0 Int64) boats cars))
                          (if (> boats 0)
                              (resume (+ (* boats 100) 9) (tuple (: 1 Int64) (: 0 Int64) cars))
                              (resume (: 0 Int64) st))))))
                 (read () st
                  (match st
                    ((tuple op boats cars)
                      (resume (+ (* op 100) (+ (* boats 10) cars)) st)))))
                (let ((a (B.car)))
                  (let ((b (B.cycle)))
                    (let ((c (B.car)))
                      (let ((d (B.boat)))
                        (let ((e (B.cycle)))
                          (let ((f (B.read)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10109901011510011 Int64))
  (call   main (: 0 Int64)) (output (: 10000020010109102 Int64)))
