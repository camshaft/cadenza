(case "sl1 a SLIDING-window fold over an enumeration: adjacent-pair deltas accumulate"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i i)))))
            (def (deltas (: ps (List (Tuple Int64 Int64))) (: prev Int64) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v)
                  (deltas t v (+ acc (- v prev))))))))
            (def (main (: n Int64))
              (deltas (Map.to-list (fill n Map.empty)) 0 0))
            (export main)))
  (call   main (: 20 Int64)) (output (: 400 Int64)))
