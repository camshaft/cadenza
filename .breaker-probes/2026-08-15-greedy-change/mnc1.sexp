(case "mnc1 a GREEDY change-making counter — pay walks the denomination ladder accumulating quotients by repeated divide-and-remainder answering the coin count, the seed decides whether a SEVEN-coin exists in the ladder, and amounts touching the seven diverge (two versus eight coins for eight cents) while seven-free amounts agree"
  (input  (do
            (effect C
              (op pay (-> Int64 Int64))
              (op drawer (-> Int64)))
            (def (coins-for (: n Int64) (: a Int64))
              (match (/ a 25)
                (q25
                  (match (- a (* q25 25))
                    (r25
                      (match (/ r25 10)
                        (q10
                          (match (- r25 (* q10 10))
                            (r10
                              (if (= n 10)
                                  (+ (+ q25 q10) (+ (/ r10 7) (- r10 (* (/ r10 7) 7))))
                                  (+ (+ q25 q10) r10)))))))))))
            (def (main (: n Int64))
              (handle C (: 0 Int64)
                ((pay (a) total
                  (match (coins-for n a)
                    (c (resume c (+ total c)))))
                 (drawer () total (resume total total)))
                (let ((a (C.pay 14)))
                  (let ((b (C.pay 8)))
                    (let ((c (C.pay 21)))
                      (let ((d (C.pay 9)))
                        (let ((e (C.pay 14)))
                          (let ((f (C.drawer)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 50203030518 Int64))
  (call   main (: 0 Int64)) (output (: 50803090530 Int64)))
