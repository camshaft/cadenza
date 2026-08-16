(case "fw1 a fold WALKS 40 keys by index, each iteration a fresh trie lookup (lookup-in-loop)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i i)))))
            (def (walk (: i Int64) (: m (Map Int64 Int64)) (: acc Int64))
              (if (= i 0) acc
                (walk (- i 1) m (+ acc (match (Map.lookup m i) ((Some v) v) ((None _u) -100000))))))
            (def (main (: n Int64))
              (walk n (fill n Map.empty) 0))
            (export main)))
  (call   main (: 40 Int64)) (output (: 22140 Int64)))
