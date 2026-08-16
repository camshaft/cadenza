(case "cx2 sr-class same-handler abort reading state advanced by CROSS-handler recursion (inner op advances OUTER)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op fin (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (B.step) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle A 0
                ((tick (u) s (resume 0 (+ s 1)))
                 (fin (u) s (* 100 s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (do (def _g (grow k)) (A.fin)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 200 Int64)))
