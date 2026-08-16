(case "sr2 a Map GROUPS its entries into a map-of-sets by value residue (the group-by idiom)"
  (input  (do
            (def (fillm (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fillm (- i 1) (Map.insert m i (% i 4)))))
            (def (group (: ps (List (Tuple Int64 Int64))) (: g (Map Int64 (Set Int64))))
              (match ps
                ((list) g)
                ((list h .. t) (match h ((tuple k v)
                  (group t (Map.insert g v
                    (match (Map.lookup g v)
                      ((Some s) (Set.insert s k))
                      ((None _u) (Set.of (list k)))))))))))
            (def (main (: n Int64))
              (do
                (def src (fillm n Map.empty))
                (def g (group (Map.to-list src) Map.empty))
                (+ (* 100 (Map.len g))
                   (match (Map.lookup g 2) ((Some s) (Set.len s)) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 410 Int64)))
