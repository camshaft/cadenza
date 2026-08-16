(case "ky1 Map.keys-shaped walk: the first tuple COMPONENT of to-list feeds a second lookup chain"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (* i 3) (* i 5)))))
            (def (chain (: ps (List (Tuple Int64 Int64))) (: m (Map Int64 Int64)) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple k _v)
                  (chain t m (+ acc (match (Map.lookup m k) ((Some v) v) ((None _u) -100000)))))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (chain (Map.to-list m) m 0)))
            (export main)))
  (call   main (: 40 Int64)) (output (: 4100 Int64)))
