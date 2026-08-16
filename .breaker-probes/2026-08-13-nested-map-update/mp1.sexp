(case "mp1 a Map VALUE that is itself a MAP updated through Map.insert-over-lookup (nested-map update idiom)"
  (input  (do
            (def (bump (: outer (Map Int64 (Map Int64 Int64))) (: ok Int64) (: ik Int64))
              (match (Map.lookup outer ok)
                ((Some inner) (Map.insert outer ok (Map.insert inner ik 1)))
                ((None _u) (Map.insert outer ok (Map.insert Map.empty ik 1)))))
            (def (main (: n Int64))
              (do
                (def m0 (bump (bump (bump Map.empty 1 10) 1 20) 2 30))
                (+ (* 100 (match (Map.lookup m0 1) ((Some i) (Map.len i)) ((None _u) -1)))
                   (+ (* 10 (match (Map.lookup m0 2) ((Some i) (Map.len i)) ((None _u) -1)))
                      (Map.len m0)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 212 Int64)))
