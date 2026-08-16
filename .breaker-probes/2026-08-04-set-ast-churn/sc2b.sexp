(case "sc2b control: order-independent Set convergence with INT elements (same shape)"
  (input  (do
            (def (up (: i Int64) (: n Int64) (: s (Set Int64)))
              (if (> i n) s (up (+ i 1) n (Set.insert s i))))
            (def (down (: i Int64) (: s (Set Int64)))
              (if (= i 0) s (down (- i 1) (Set.insert s i))))
            (def (main (: n Int64))
              (if (= (up 1 n (Set.of (list))) (down n (Set.of (list)))) 1 0))
            (export main)))
  (call   main (: 100 Int64)) (output (: 1 Int64)))
