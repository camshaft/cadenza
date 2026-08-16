(case "rw1 rx6-adjacent under the landed blunt guard: no-observer depth-2 recursion (should be untouched)"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1))))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (loop 2))))
            (export main)))
  (output (: 21 Int64)))
