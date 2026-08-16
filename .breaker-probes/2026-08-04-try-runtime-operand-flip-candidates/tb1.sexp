(case "tb1 a try-unwrapped List.at whose INDEX comes from a trie lookup (per-op try x champ)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (+ i 1)))))
            (def (pick (: xs (List Int64)) (: m (Map Int64 Int64)) (: k Int64))
              (let ((idx (try (Map.lookup m k))))
                (let ((v (try (List.at xs idx))))
                  (Some v))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def xs (list 10 20 30))
                (+ (* 10 (match (pick xs m 1) ((Some v) v) ((None _u) -1)))
                   (match (pick xs m 99) ((Some _v) 0) ((None _u) 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 301 Int64)))
