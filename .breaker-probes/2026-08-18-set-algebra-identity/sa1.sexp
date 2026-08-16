(case "sa1 self-difference of a 100-element trie is the canonical empty set"
  (input  (do
            (def (build (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc (build (- i 1) (Set.insert acc i))))
            (def (main (: n Int64))
              (do
                (def s (build n (Set.of (list))))
                (def d (Set.difference s s))
                (+ (* 10 (if (= d (Set.of (list))) 1 0)) (Set.len d))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 10 Int64)))
