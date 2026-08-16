(case "xe1 TWO outer op-results as SIBLING args of one inner perform"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((put (v w) s (resume (+ v w) s)))
                  (B.put (A.get) (A.get)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 15 Int64)))
