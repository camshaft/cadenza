(case "cx1 sr-class cross-handler face: recursive INNER performs, then an OUTER-effect ABORT reads outer state"
  (input  (do
            (effect A (op fin (-> Unit Int64)))
            (effect B (op put (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (B.put) (grow (- n 1)))))
            (def (main (: k Int64))
              (handle A 7
                ((fin (u) s (* 100 s)))
                (handle B 0
                  ((put (u) t (resume 0 (+ t 1))))
                  (do (def _g (grow k)) (A.fin)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 700 Int64)))
