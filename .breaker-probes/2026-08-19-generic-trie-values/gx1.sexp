(case "gx1 an unannotated generic helper serves trie VALUES of two element types in one program"
  (input  (do
            (def (first-or xs d)
              (match (List.at xs 0) ((Some v) v) ((None _u) d)))
            (def (main (: n Int64))
              (do
                (def mi (Map.insert Map.empty 1 (list n (* n 2))))
                (def ms (Map.insert Map.empty 1 (list "alpha" "beta")))
                (+ (* 10 (match (Map.lookup mi 1) ((Some xs) (first-or xs -1)) ((None _u) -2)))
                   (match (Map.lookup ms 1) ((Some xs) (String.byte-len (first-or xs "?"))) ((None _u) -2)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 75 Int64)))
