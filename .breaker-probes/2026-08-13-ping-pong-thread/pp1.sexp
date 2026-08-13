(case "pp1 a PING-PONG data thread across two handlers — A's answer feeds B's argument and B's answer feeds A's next argument, additive and modular-multiplicative arms advance independently"
  (input  (do
            (effect A (op step (-> Int64 Int64)))
            (effect B (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((step (v) s (resume (+ s v) (+ s 1))))
                (handle B 3
                  ((step (v) t (resume (% (* v t) 97) (+ t 2))))
                  (let ((x1 (A.step 1)))
                    (let ((y1 (B.step x1)))
                      (let ((x2 (A.step y1)))
                        (let ((y2 (B.step x2)))
                          (+ (* 100 (+ (* 1000 (+ (* 100 x1) y1)) x2)) y2))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 41201680 Int64))
  (call   main (: 50 Int64)) (output (: 515610750 Int64)))
