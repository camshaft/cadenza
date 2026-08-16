(case "mv1 a trie of 40 entries with LIST values keeps each heap value addressable at depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (List Int64))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (list i (* i 2) (* i 3))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (match (Map.lookup m 25)
                  ((Some xs) (+ (* 10 (List.len xs))
                                (match (List.at xs 2) ((Some v) v) ((None _u) -1))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 105 Int64)))
