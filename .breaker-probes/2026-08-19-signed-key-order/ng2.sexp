(case "ng2 a negative-and-positive-key trie churned back keys and enumerates like the direct build"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m (- 0 (* i 3)) i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m (- 0 (* i 3))))))
            (def (main (: n Int64))
              (do
                (def direct (Map.insert (Map.insert Map.empty -7 70) 7 77))
                (def churned (shrink 1 n (grow 1 n direct)))
                (+ (* 10 (if (= (Map.to-list churned) (Map.to-list direct)) 1 0))
                   (match (Map.lookup churned -7) ((Some v) (if (= v 70) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 11 Int64)))
