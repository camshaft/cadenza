(case "em3 a SET churned to empty equals a fresh empty set and re-accepts elements"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: s (Set Int64)))
              (if (= i n) s (grow (+ i 1) n (Set.insert s i))))
            (def (shrink (: i Int64) (: n Int64) (: s (Set Int64)))
              (if (= i n) s (shrink (+ i 1) n (Set.remove s i))))
            (def (main (: n Int64))
              (do
                (def emptied (shrink 1 n (grow 1 n (Set.of (list)))))
                (+ (* 100 (if (= emptied (Set.of (list))) 1 0))
                   (+ (* 10 (Set.len emptied))
                      (if (Set.contains (Set.insert emptied 42) 42) 1 0)))))
            (export main)))
  (call   main (: 120 Int64)) (output (: 101 Int64)))
