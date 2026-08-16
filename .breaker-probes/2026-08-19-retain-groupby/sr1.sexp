(case "sr1 a Set DIFFERENCE walk feeding a Map rebuild (the retain-absent idiom)"
  (input  (do
            (def (fillm (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fillm (- i 1) (Map.insert m i (* i 2)))))
            (def (fills (: i Int64) (: s (Set Int64)))
              (if (= i 0) s (fills (- i 1) (if (= (% i 3) 0) (Set.insert s i) s))))
            (def (retain (: ps (List (Tuple Int64 Int64))) (: dead (Set Int64)) (: m (Map Int64 Int64)))
              (match ps
                ((list) m)
                ((list h .. t) (match h ((tuple k v)
                  (retain t dead (if (Set.contains dead k) m (Map.insert m k v))))))))
            (def (main (: n Int64))
              (do
                (def src (fillm n Map.empty))
                (def dead (fills n (Set.of (list))))
                (def kept (retain (Map.to-list src) dead Map.empty))
                (+ (* 10 (Map.len kept))
                   (match (Map.lookup kept 9) ((Some _v) 0) ((None _u) 1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 271 Int64)))
