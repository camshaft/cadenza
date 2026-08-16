(case "gc1 interleaved insert/remove CHURN keeps a persistent snapshot intact at every generation"
  (input  (do
            (def (churn (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m
                  (churn (+ i 1) n
                    (if (= (% i 3) 0)
                        (Map.remove m (- i 2))
                        (Map.insert m i (* i 10))))))
            (def (main (: n Int64))
              (do
                (def base (Map.insert (Map.insert Map.empty 1000 1) 2000 2))
                (def churned (churn 1 n base))
                (+ (* 100 (Map.len base))
                   (match (Map.lookup churned 1000) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 201 Int64)))
