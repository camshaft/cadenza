(case "lpc1 a LAMPORT CLOCK — local events tick the counter, receives jump it to max(local,remote)+1; the stale remote (already-past timestamp) still ticks by one through the max"
  (input  (do
            (effect S
              (op event (-> Int64))
              (op recv (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S 0
                ((event () c (resume (+ c 1) (+ c 1)))
                 (recv (ts) c
                  (let ((c2 (+ (if (> ts c) ts c) 1)))
                    (resume c2 c2))))
                (let ((a (S.event)))
                  (let ((b (S.recv n)))
                    (let ((c (S.event)))
                      (let ((d (S.recv 2)))
                        (let ((e (S.event)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106070809 Int64))
  (call   main (: 0 Int64)) (output (: 102030405 Int64)))
