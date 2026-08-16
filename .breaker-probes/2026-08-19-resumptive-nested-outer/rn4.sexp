(case "rn4 MATCH-arm resume-value: the outer perform sits in a match inside the nested arm"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Int64 Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (B.step n) (loop (- n 1)))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (v) t (resume (match v (0 99) (_ (A.tick))) t)))
                  (+ (loop 1) (A.get)))))
            (export main)))
  (output (: 21 Int64)))
