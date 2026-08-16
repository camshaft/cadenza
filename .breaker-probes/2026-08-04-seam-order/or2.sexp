(case "or2 Map.to-list orders by KEY across the representation seam with mixed-magnitude keys"
  (input  (do
            (def (main (: z Int64))
              (match (Map.to-list (Map.insert (Map.insert (Map.insert Map.empty (+ z 536870920) 30) (- 0 (+ z 5)) 10) (+ z 7) 20))
                ((list (tuple _k1 v1) (tuple _k2 v2) (tuple _k3 v3))
                  (+ (* 100 v1) (+ (* 10 (/ v2 10)) (/ v3 10))))
                (_other -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1023 Int64)))
