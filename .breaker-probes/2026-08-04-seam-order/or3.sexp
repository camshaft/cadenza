(case "or3 fold over Set.to-list across the seam is deterministic and matches the sorted sum"
  (input  (do
            (def (sum-list (: xs (List Int64)) (: acc Int64))
              (match xs ((list) acc) ((list h .. t) (sum-list t (+ (* 2 acc) h)))))
            (def (main (: z Int64))
              (sum-list (Set.to-list (Set.of (list (+ z 536870919) (+ z 2) (+ z 536870912)))) 0))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1610612758 Int64)))
