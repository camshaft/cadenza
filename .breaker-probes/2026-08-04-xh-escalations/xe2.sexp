(case "xe2 NESTED op-arg: the inner perform's arg is ANOTHER inner perform whose arg is the outer op-result"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s s)))
                (handle B 0
                  ((put (v) s (resume (+ v 1) (+ s 1))))
                  (B.put (B.put (A.get))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 9 Int64)))
