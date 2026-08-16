(case "rz1 the rn drop's NON-recursive control family: helper-call depth without recursion stays correct"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (once) (B.step))
            (def (twice) (+ (once) (once)))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (+ (twice) (A.get)))))
            (export main)))
  (output (: 33 Int64)))
