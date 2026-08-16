(case "rn6 accum-DISABLED variant 2: NON-linear accumulator position (mult not add)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (= n 0) 1 (* (+ (B.step) 1) (walk (- n 1)))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (+ (walk 1) (A.get)))))
            (export main)))
  (output (: 22 Int64)))
