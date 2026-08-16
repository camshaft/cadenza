(case "nu1 a NEWTYPE-tagged key preserves distinctness from its raw inner at trie depth"
  (input  (do
            (type UserId (Mk Int64))
            (def (fill (: i Int64) (: m (Map UserId Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (UserId.Mk i) (* i 5)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (UserId.Mk 20)) ((Some v) (if (= v 100) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 401 Int64)))
