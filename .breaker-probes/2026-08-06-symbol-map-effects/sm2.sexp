(case "sm2 a nested Map-of-Map state — the arm updates the INNER map through the outer"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Map.insert Map.empty 1 (Map.insert Map.empty 2 10))
                ((put (v) s
                  (resume v
                    (match (Map.lookup s 1)
                      ((Some inner) (Map.insert s 1 (Map.insert inner 2 (+ v (match (Map.lookup inner 2) ((Some x) x) ((None _u) 0))))))
                      ((None _u) s))))
                 (get (u) s
                  (resume (match (Map.lookup s 1)
                            ((Some inner) (match (Map.lookup inner 2) ((Some x) x) ((None _u) -1)))
                            ((None _u) -2)) s)))
                (+ (St.put n) (+ (St.put 7) (* 100 (St.get))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2212 Int64)))
