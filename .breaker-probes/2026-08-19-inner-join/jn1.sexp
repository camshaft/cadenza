(case "jn1 an INNER-JOIN materialization: matched pairs from two tries build a result trie"
  (input  (do
            (def (filla (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (filla (- i 1) (Map.insert m i (* i 2)))))
            (def (fillb (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fillb (- i 1) (if (= (% i 3) 0) (Map.insert m i (* i 5)) m))))
            (def (join (: ps (List (Tuple Int64 Int64))) (: b (Map Int64 Int64)) (: out (Map Int64 Int64)))
              (match ps
                ((list) out)
                ((list h .. t) (match h ((tuple k va)
                  (join t b (match (Map.lookup b k)
                              ((Some vb) (Map.insert out k (+ va vb)))
                              ((None _u) out))))))))
            (def (main (: n Int64))
              (do
                (def a (filla n Map.empty))
                (def b (fillb n Map.empty))
                (def joined (join (Map.to-list a) b Map.empty))
                (+ (* 100 (Map.len joined))
                   (match (Map.lookup joined 12) ((Some v) (if (= v 84) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 1001 Int64)))
