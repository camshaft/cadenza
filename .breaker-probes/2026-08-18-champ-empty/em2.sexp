(case "em2 the churned-to-empty map re-grows correctly (no tombstone corruption)"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m i (* i 2)))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m i))))
            (def (main (: n Int64))
              (do
                (def emptied (shrink 1 n (grow 1 n Map.empty)))
                (def again (Map.insert (Map.insert emptied 7 70) 8 80))
                (+ (* 100 (Map.len again))
                   (match (Map.lookup again 7) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 120 Int64)) (output (: 270 Int64)))
