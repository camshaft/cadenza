(case "em1 a map churned down to EMPTY equals Map.empty (canonical empty identity)"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m i i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m i))))
            (def (main (: n Int64))
              (do
                (def emptied (shrink 1 n (grow 1 n Map.empty)))
                (+ (* 10 (if (= emptied Map.empty) 1 0))
                   (if (= (Map.len emptied) 0) 1 0))))
            (export main)))
  (call   main (: 120 Int64)) (output (: 11 Int64)))
