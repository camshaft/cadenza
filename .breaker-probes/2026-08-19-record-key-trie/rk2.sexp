(case "rk2 a record-keyed trie churned back equals the direct build (compound-key history-independence)"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map (Record (x Int64) (y Int64)) Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m (record (x (+ i 100)) (y i)) i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map (Record (x Int64) (y Int64)) Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m (record (x (+ i 100)) (y i))))))
            (def (main (: n Int64))
              (do
                (def direct (Map.insert Map.empty (record (x 7) (y 8)) 50))
                (def churned (shrink 1 n (grow 1 n direct)))
                (+ (* 10 (if (= churned direct) 1 0))
                   (match (Map.lookup churned (record (x 7) (y 8))) ((Some v) (if (= v 50) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 11 Int64)))
