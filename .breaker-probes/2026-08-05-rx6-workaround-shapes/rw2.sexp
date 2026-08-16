(case "rw2 blunt-guard band: NON-recursive depth-3 chain with observer (rz1 twin, must still fold)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (effect C (op hop (-> Unit Int64)))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (handle C 0
                    ((hop (u) w (resume (B.step) w)))
                    (+ (+ (C.hop) (C.hop)) (A.get))))))
            (export main)))
  (output (: 33 Int64)))
