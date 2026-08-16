(case "gc3 the churned-back map IS a CHAMP key equal to the direct build (history-independence as key)"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m i i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m i))))
            (def (main (: n Int64))
              (do
                (def back (shrink 1 n (grow 1 n (Map.insert Map.empty 999 9))))
                (match (Map.lookup (Map.insert Map.empty (Map.insert Map.empty 999 9) 42) back)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 150 Int64)) (output (: 42 Int64)))
