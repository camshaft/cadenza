(case "rv1 Record.with on a record stored as a deep-trie VALUE rebuilds only that entry"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (Record (a Int64) (b Int64)))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (record (a i) (b (* i 2)))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def m2 (match (Map.lookup m 25)
                          ((Some r) (Map.insert m 25 (Record.with r #"a" 999)))
                          ((None _u) m)))
                (+ (* 1000 (match (Map.lookup m2 25) ((Some r) (. r a)) ((None _u) -1)))
                   (+ (match (Map.lookup m2 24) ((Some r) (. r a)) ((None _u) -1))
                      (match (Map.lookup m 25) ((Some r) (. r a)) ((None _u) -1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 999049 Int64)))
