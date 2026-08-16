(case "vlt1 a TIME-LOCK vault with a duress code — the correct code opens ONLY at timer zero (a nested-if inside the code dispatch), the duress code counts an alarm and PENALIZES the timer by two, ticks count down to a floor answering timer and lock state, status packs alarms timer and lock, and the seed's initial timer decides whether the final correct entry finds zero (vault opens) or the duress penalty still holding it shut"
  (input  (do
            (effect V
              (op enter (-> Int64 Int64))
              (op tick (-> Int64))
              (op status (-> Int64)))
            (def (main (: n Int64))
              (handle V (tuple (: 1 Int64) (+ (: 1 Int64) (* (% n 3) 2)) (: 0 Int64))
                ((enter (code) st
                  (match st
                    ((tuple locked timer alarms)
                      (if (= code 7)
                          (if (= timer 0)
                              (resume (: 111 Int64) (tuple (: 0 Int64) timer alarms))
                              (resume (+ (: 200 Int64) timer) st))
                          (if (= code 9)
                              (resume (+ (: 900 Int64) (+ alarms 1))
                                      (tuple locked (+ timer 2) (+ alarms 1)))
                              (resume (+ (: 400 Int64) (% code 10)) st))))))
                 (tick () st
                  (match st
                    ((tuple locked timer alarms)
                      (if (> timer 0)
                          (resume (+ (* (- timer 1) 10) locked) (tuple locked (- timer 1) alarms))
                          (resume (* 10 0) st)))))
                 (status () st
                  (match st
                    ((tuple locked timer alarms)
                      (resume (+ (* alarms 100) (+ (* timer 10) locked)) st)))))
                (let ((a (V.tick)))
                  (let ((b (V.enter (: 9 Int64))))
                    (let ((c (V.tick)))
                      (let ((d (V.tick)))
                        (let ((e (V.enter (: 7 Int64))))
                          (let ((f (V.status)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 21901031021202121 Int64))
  (call   main (: 0 Int64)) (output (: 1901011001111100 Int64)))
