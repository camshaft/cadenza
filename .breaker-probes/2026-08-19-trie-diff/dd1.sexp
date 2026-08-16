(case "dd1 a DIFF of two tries: entries in A absent-or-different in B (the reconciliation walk)"
  (input  (do
            (def (filla (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (filla (- i 1) (Map.insert m i (* i 2)))))
            (def (fillb (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fillb (- i 1) (Map.insert m i (if (= (% i 5) 0) (* i 3) (* i 2))))))
            (def (diff (: ps (List (Tuple Int64 Int64))) (: b (Map Int64 Int64)) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple k v)
                  (diff t b (+ acc (match (Map.lookup b k)
                                     ((Some w) (if (= w v) 0 1))
                                     ((None _u) 1)))))))))
            (def (main (: n Int64))
              (do
                (def a (filla n Map.empty))
                (def b (fillb (- n 5) Map.empty))
                (diff (Map.to-list a) b 0)))
            (export main)))
  (call   main (: 30 Int64)) (output (: 10 Int64)))
