(case "hs3 TWO Map-stated handlers stacked — each op routes to its own Map, no cross-contamination"
  (input  (do
            (effect A (op puta (-> Int64 Unit)) (op sizea (-> Unit Int64)))
            (effect B (op putb (-> Int64 Unit)) (op sizeb (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A (Map.empty)
                ( (puta (k) m (resume unit (Map.insert m k k)))
                  (sizea (u) m (resume (Map.len m) m)) )
                (handle B (Map.empty)
                  ( (putb (k) m (resume unit (Map.insert m k k)))
                    (sizeb (u) m (resume (Map.len m) m)) )
                  (do
                    (A.puta 1) (B.putb 10) (A.puta 2) (B.putb 20) (B.putb 30) (A.puta n)
                    (+ (* 10 (A.sizea)) (B.sizeb))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 33 Int64))
  (call   main (: 1 Int64)) (output (: 23 Int64)))
