(case "rx4 blast radius: DEPTH-4 chain + recursion (does the drop deepen?)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (effect C (op hop (-> Unit Int64)))
            (effect D (op jmp (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (D.jmp) (loop (- n 1)))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (handle C 0
                    ((hop (u) w (resume (B.step) w)))
                    (handle D 0
                      ((jmp (u) x (resume (C.hop) x)))
                      (+ (loop 1) (A.get)))))))
            (export main)))
  (output (: 21 Int64)))
