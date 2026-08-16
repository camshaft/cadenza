(case "trf1 a TRAFFIC LIGHT with a pedestrian button — tick counts the phase timer down and rolls green to yellow to red to green with fresh timers, a press LATCHES only while green and only when the seed enables the button, the latch buys a LONGER yellow consumed at the next green expiry, and every answer packs phase timer and latch; the latched run's phase boundaries all shift one tick late"
  (input  (do
            (effect L
              (op tick (-> Int64))
              (op press (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 3 Int64) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple ph tm held)
                      (if (= (- tm 1) 0)
                          (if (= ph 0)
                              (resume (+ (* 100 1) (+ (* 10 (+ 1 held)) 0))
                                      (tuple (: 1 Int64) (+ 1 held) (: 0 Int64)))
                              (if (= ph 1)
                                  (resume (+ (* 100 2) (+ (* 10 2) held))
                                          (tuple (: 2 Int64) (: 2 Int64) held))
                                  (resume (+ (* 100 0) (+ (* 10 3) held))
                                          (tuple (: 0 Int64) (: 3 Int64) held))))
                          (resume (+ (* 100 ph) (+ (* 10 (- tm 1)) held))
                                  (tuple ph (- tm 1) held))))))
                 (press () st
                  (match st
                    ((tuple ph tm held)
                      (if (if (= ph 0) (not (= (% n 3) 0)) false)
                          (resume (+ (* 100 ph) (+ (* 10 tm) 1))
                                  (tuple ph tm (: 1 Int64)))
                          (resume (+ (* 100 ph) (+ (* 10 tm) held)) st))))))
                (let ((a (L.tick)))
                  (let ((b (L.press)))
                    (let ((c (L.tick)))
                      (let ((d (L.tick)))
                        (let ((e (L.tick)))
                          (let ((f (L.tick)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 20021011120110220 Int64))
  (call   main (: 0 Int64)) (output (: 20020010110220210 Int64)))
