(case "tp1 a TOPOLOGICAL-style layered walk: dependency counts drain level by level"
  (input  (do
            (def (deps (: k Int64)) (if (< k 2) 0 (if (< k 4) 1 2)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (deps i)))))
            (def (ready (: ps (List (Tuple Int64 Int64))) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple k d)
                  (ready t (if (= d 0) (+ acc k) acc)))))))
            (def (drain (: m (Map Int64 Int64)) (: rounds Int64) (: total Int64))
              (if (= rounds 0) total
                (do
                  (def r (ready (Map.to-list m) 0))
                  (def m2 (compact (Map.to-list m) m))
                  (drain m2 (- rounds 1) (+ total r)))))
            (def (compact (: ps (List (Tuple Int64 Int64))) (: m (Map Int64 Int64)))
              (match ps
                ((list) m)
                ((list h .. t) (match h ((tuple k d)
                  (compact t (if (= d 0) (Map.remove m k) (Map.insert m k (- d 1)))))))))
            (def (main (: n Int64))
              (drain (fill n Map.empty) 3 0))
            (export main)))
  (call   main (: 6 Int64)) (output (: 21 Int64)))
