(case "st2 Set.to-list over a 100-element trie enumerates strictly increasing all the way"
  (input  (do
            (def (build (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc (build (- i 1) (Set.insert acc (* i 13)))))
            (def (inc (: xs (List Int64)) (: prev Int64) (: cnt Int64))
              (match xs
                ((list) cnt)
                ((list h .. t) (if (> h prev) (inc t h (+ cnt 1)) -100000))))
            (def (main (: n Int64))
              (inc (Set.to-list (build n (Set.of (list)))) -1 0))
            (export main)))
  (call   main (: 100 Int64)) (output (: 100 Int64)))
