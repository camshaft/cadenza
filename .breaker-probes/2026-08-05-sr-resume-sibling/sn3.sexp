(case "sn3 the abort observer at the base of a SECOND recursion after grow (abort deep in a later callee)"
  (input  (do
            (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
            (def (grow (: n Int64))
              (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
            (def (walkq (: n Int64))
              (if (= n 0) (Acc.fin) (+ 0 (walkq (- n 1)))))
            (def (main (: k Int64))
              (handle Acc 0
                ((put (u) s (resume 0 (+ s 1)))
                 (fin (u) s s))
                (do (def _g (grow k)) (walkq 3))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3 Int64)))
