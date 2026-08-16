(case "dt1 a 40-key trie of TUPLE keys (Int,Int) resolves compound-key descent at depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map (Tuple Int64 Int64) Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (tuple (% i 7) (/ i 7)) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (tuple 3 3)) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 424 Int64)))
