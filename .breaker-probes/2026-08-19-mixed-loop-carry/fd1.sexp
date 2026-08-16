(case "fd1 a FOLD threads a trie AND a scalar accumulator in one recursion (mixed loop-carry)"
  (input  (do
            (def (walk (: i Int64) (: m (Map Int64 Int64)) (: sum Int64))
              (if (= i 0) (tuple m sum)
                (walk (- i 1) (Map.insert m i (* i 5)) (+ sum i))))
            (def (main (: n Int64))
              (match (walk n Map.empty 0)
                ((tuple m sum)
                  (+ (* 100 (Map.len m))
                     (+ sum
                        (match (Map.lookup m 30) ((Some v) (if (= v 150) 1000 0)) ((None _u) -1)))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 5820 Int64)))
