(case "m2m1 a MAP-OF-MAPS handler state — each dispatch bumps a (category key) cell, the drain reads across both levels"
  (input  (do
            (effect Tally
              (op bump (-> Int64 Int64 Int64))
              (op read (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle Tally (Map.insert Map.empty 1 (Map.insert Map.empty 10 n))
                ((bump (k j) s
                  (let ((inner (match (Map.lookup s k) ((Some im) im) ((None u) Map.empty))))
                    (let ((old (match (Map.lookup inner j) ((Some v) v) ((None u) 0))))
                      (resume old (Map.insert s k (Map.insert inner j (+ old 1)))))))
                 (read (k j) s
                  (resume (match (Map.lookup s k)
                            ((Some im) (match (Map.lookup im j) ((Some v) v) ((None u) -1)))
                            ((None u) -2))
                          s)))
                (let ((a (Tally.bump 1 10)))
                  (let ((b (Tally.bump 2 20)))
                    (let ((c (Tally.bump 2 20)))
                      (+ (* 1000000 a)
                         (+ (* 10000 b)
                            (+ (* 100 c)
                               (+ (* 10 (Tally.read 1 10)) (Tally.read 2 20))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5000162 Int64))
  (call   main (: 0 Int64)) (output (: 112 Int64)))
