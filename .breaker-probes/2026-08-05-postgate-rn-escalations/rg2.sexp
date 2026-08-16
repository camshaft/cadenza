(case "rg2 observer INSIDE the recursion alongside the chain-perform ((+ (C.hop) (A.get) ...) per iteration)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (effect C (op hop (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (C.hop) (+ (A.get) (loop (- n 1))))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (handle C 0
                    ((hop (u) w (resume (B.step) w)))
                    (loop 1)))))
            (export main)))
  (output (: 21 Int64)))
