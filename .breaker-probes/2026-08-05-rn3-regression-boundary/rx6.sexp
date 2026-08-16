(case "rx6 NO-observer face: recursion x depth-3 loop sum alone (n=2, no A.get)"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (effect C (op hop (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (C.hop) (loop (- n 1)))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1))))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (handle C 0
                    ((hop (u) w (resume (B.step) w)))
                    (loop 2)))))
            (export main)))
  (output (: 21 Int64)))
