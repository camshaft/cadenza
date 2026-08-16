(case "pp1 a PING-PONG volley — each hop feeds A's answer into B's argument and back, both states stride independently"
  (input  (do
            (effect A (op pa (-> Int64 Int64)))
            (effect B (op pb (-> Int64 Int64)))
            (def (volley (: k Int64) (: ball Int64))
              (if (< k 1) ball
                (volley (- k 1) (B.pb (A.pa ball)))))
            (def (main (: n Int64))
              (handle A 0
                ((pa (v) s (resume (+ (* 2 v) s) (+ s 1))))
                (handle B 100
                  ((pb (v) t (resume (+ v t) (+ t 10))))
                  (volley n 1))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 752 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
