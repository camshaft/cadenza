(case "sa2 union with the self-difference empty is identity at trie scale"
  (input  (do
            (def (build (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc (build (- i 1) (Set.insert acc i))))
            (def (main (: n Int64))
              (do
                (def s (build n (Set.of (list))))
                (def rt (Set.union (Set.difference s s) s))
                (+ (* 10 (if (= rt s) 1 0)) (if (Set.contains rt 57) 1 0))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 11 Int64)))
