(case "st1 Set.to-list rebuilt via Set.of round-trips a 100-element trie to the identical set"
  (input  (do
            (def (build (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc (build (- i 1) (Set.insert acc (* i 11)))))
            (def (main (: n Int64))
              (do
                (def src (build n (Set.of (list))))
                (def rt (Set.of (Set.to-list src)))
                (+ (* 10 (if (= rt src) 1 0)) (if (= (Set.len rt) n) 1 0))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 11 Int64)))
