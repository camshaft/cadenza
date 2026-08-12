(case "mrv1 REMOVE-then-REINSERT churn on a Map state — del answers the removed value (0 when absent); for n=98 the second del hits the n+1 key it planted"
  (input  (do
            (effect S
              (op put (-> Int64 Int64 Int64))
              (op del (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (let ((m2 (Map.insert m k v)))
                    (resume (Map.len m2) m2)))
                 (del (k) m
                  (resume (match (Map.lookup m k) ((Some x) x) ((None u) 0))
                          (Map.remove m k))))
                (let ((a (S.put n 5)))
                  (let ((b (S.put (+ n 1) 7)))
                    (let ((c (S.del n)))
                      (let ((d (S.put n 9)))
                        (let ((e (S.del 99)))
                          (+ (* 10 (+ (* 100 (+ (* 10 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 125020 Int64))
  (call   main (: 98 Int64)) (output (: 125027 Int64)))
