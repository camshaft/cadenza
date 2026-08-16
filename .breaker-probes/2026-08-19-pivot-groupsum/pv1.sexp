(case "pv1 a PIVOT walk: rows of (group, value) tuples aggregate into a group-sum trie"
  (input  (do
            (def (rows (: i Int64) (: acc (List (Tuple Int64 Int64))))
              (if (= i 0) acc (rows (- i 1) (List.push acc (tuple (% i 4) (* i 10))))))
            (def (pivot (: xs (List (Tuple Int64 Int64))) (: g (Map Int64 Int64)))
              (match xs
                ((list) g)
                ((list h .. t) (match h ((tuple grp v)
                  (pivot t (Map.insert g grp
                    (+ v (match (Map.lookup g grp) ((Some s) s) ((None _u) 0))))))))))
            (def (main (: n Int64))
              (do
                (def g (pivot (rows n (list)) Map.empty))
                (+ (* 10000 (Map.len g))
                   (match (Map.lookup g 2) ((Some s) s) ((None _u) -1)))))
            (export main)))
  (call   main (: 24 Int64)) (output (: 40720 Int64)))
