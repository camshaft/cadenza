(case "iw2 a map MIGRATION: drain one trie entry-by-entry INTO another in a single walk"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (migrate (: ps (List (Tuple Int64 Int64))) (: src (Map Int64 Int64)) (: dst (Map Int64 Int64)))
              (match ps
                ((list) (tuple src dst))
                ((list h .. t) (match h ((tuple k v)
                  (migrate t (Map.remove src k) (Map.insert dst k (* v 10))))))))
            (def (main (: n Int64))
              (match (migrate (Map.to-list (fill n Map.empty)) (fill n Map.empty) Map.empty)
                ((tuple src dst)
                  (+ (* 1000 (Map.len src))
                     (+ (Map.len dst)
                        (match (Map.lookup dst 15) ((Some v) (if (= v 150) 100 0)) ((None _u) -1)))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 140 Int64)))
