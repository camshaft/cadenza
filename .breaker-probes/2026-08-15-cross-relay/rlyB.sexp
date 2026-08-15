(case "rlyB single-use arm-perform control (answer only, state untouched) — B's handler arm PERFORMS the outer A (same op name, same signature) doubling the argument and folding A's answer into B's state, so every relay draw advances BOTH threads and the identity question routes THROUGH an arm-perform under schema-hash-only dispatch"
  (input  (do
            (effect A (op bump (-> Int64 Int64)))
            (effect B (op bump (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A (: n Int64)
                ((bump (v) s (resume (+ s v) (+ s v))))
                (handle B (: 0 Int64)
                  ((bump (v) s
                    (resume (A.bump (* v 2)) s)))
                  (let ((a (A.bump 1)))
                    (let ((b (B.bump 2)))
                      (let ((c (A.bump 1)))
                        (let ((d (B.bump 3)))
                          (let ((e (A.bump 1)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11015016022023 Int64))
  (call   main (: 0 Int64)) (output (: 1005006012013 Int64)))
