(case "uw2 a list of MAPS: List.update swaps one trie for another leaving neighbors live"
  (input  (do
            (def (fill (: i Int64) (: k Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) k (Map.insert m (+ (* k 100) i) i))))
            (def (build (: j Int64) (: acc (List (Map Int64 Int64))))
              (if (= j 0) acc (build (- j 1) (List.push acc (fill 10 j Map.empty)))))
            (def (main (: n Int64))
              (do
                (def xs (build n (list)))
                (def ys (List.update xs 3 (fill 25 99 Map.empty)))
                (+ (* 100 (match (List.at ys 3) ((Some m) (Map.len m)) ((None _u) -1)))
                   (+ (* 10 (match (List.at ys 2) ((Some m) (Map.len m)) ((None _u) -1)))
                      (match (List.at xs 3) ((Some m) (Map.len m)) ((None _u) -1))))))
            (export main)))
  (call   main (: 8 Int64)) (output (: 2610 Int64)))
