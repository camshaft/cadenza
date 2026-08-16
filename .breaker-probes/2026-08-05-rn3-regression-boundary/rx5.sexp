(case "rx5 blast radius: MUTUAL recursion (two fns alternating) through the depth-3 chain"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (effect C (op hop (-> Unit Int64)))
            (def (even-walk (: n Int64))
              (if (= n 0) 0 (+ (C.hop) (odd-walk (- n 1)))))
            (def (odd-walk (: n Int64))
              (if (= n 0) 0 (+ (C.hop) (even-walk (- n 1)))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (handle C 0
                    ((hop (u) w (resume (B.step) w)))
                    (+ (even-walk 1) (A.get))))))
            (export main)))
  (output (: 21 Int64)))
