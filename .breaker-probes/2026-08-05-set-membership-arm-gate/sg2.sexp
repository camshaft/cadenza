(case "sg2 the enclosing capture is a MAP and the arm both READS it and threads a hit-count state"
  (input  (do
            (effect St (op price (-> Int64 Int64)) (op hits (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert (Map.insert Map.empty 1 100) 2 250))
                (handle St 0
                  ((price (k) s
                    (match (Map.lookup table k)
                      ((Some v) (resume v (+ s 1)))
                      ((None _u) (resume 0 s))))
                   (hits (u) s (resume s s)))
                  (+ (St.price 1) (+ (St.price 7) (+ (St.price 2) (* 1000 (St.hits))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2350 Int64)))
