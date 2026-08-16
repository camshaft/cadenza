(case "tv1 Set.to-list of a set of TUPLES feeds a fold destructuring each compound element"
  (input  (do
            (def (build (: i Int64) (: acc (Set (Tuple Int64 Int64))))
              (if (= i 0) acc (build (- i 1) (Set.insert acc (tuple (% i 5) i)))))
            (def (net (: xs (List (Tuple Int64 Int64))) (: acc Int64))
              (match xs
                ((list) acc)
                ((list h .. t) (match h ((tuple g v) (net t (+ acc (* g v))))))))
            (def (main (: n Int64))
              (do
                (def s (build n (Set.of (list))))
                (+ (* 10000 (Set.len s)) (net (Set.to-list s) 0))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 300930 Int64)))
