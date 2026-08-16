(case "sa3 intersection of a trie with its churned-back twin is the twin itself (canonical inputs commute)"
  (input  (do
            (def (build (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc (build (- i 1) (Set.insert acc i))))
            (def (drop-half (: i Int64) (: s (Set Int64)))
              (if (> i 100) s (drop-half (+ i 2) (Set.remove s i))))
            (def (main (: n Int64))
              (do
                (def full (build n (Set.of (list))))
                (def odds (drop-half 2 full))
                (def inter (Set.intersection full odds))
                (+ (* 10 (if (= inter odds) 1 0)) (if (Set.contains inter 2) 1 0))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 10 Int64)))
