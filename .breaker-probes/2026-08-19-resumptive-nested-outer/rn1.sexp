(case "rn1 v-effects find: recursive performer of a nested op whose resume performs the OUTER effect"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (+ (loop 1) (A.get)))))
            (export main)))
  (output (: 21 Int64)))
