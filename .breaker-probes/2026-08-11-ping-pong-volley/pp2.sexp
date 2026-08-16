(case "pp2 the volley re-instantiates the INNER handler per hop — a fresh B shadow seeded by the loop counter wraps each exchange"
  (input  (do
            (effect A (op pa (-> Int64 Int64)))
            (effect B (op pb (-> Int64 Int64)))
            (def (volley (: k Int64) (: ball Int64))
              (if (< k 1) ball
                (volley (- k 1)
                  (handle B (* k 100)
                    ((pb (v) t (resume (+ v t) t)))
                    (B.pb (A.pa ball))))))
            (def (main (: n Int64))
              (handle A 0
                ((pa (v) s (resume (+ (* 2 v) s) (+ s 1))))
                (volley n 1)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1712 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
