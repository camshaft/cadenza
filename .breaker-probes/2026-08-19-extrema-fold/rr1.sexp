(case "rr1 a fold REDUCES an enumeration to min and max in one walk (paired extrema)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (% (* i 37) 100)))))
            (def (extrema (: ps (List (Tuple Int64 Int64))) (: lo Int64) (: hi Int64))
              (match ps
                ((list) (tuple lo hi))
                ((list h .. t) (match h ((tuple _k v)
                  (extrema t (if (< v lo) v lo) (if (> v hi) v hi)))))))
            (def (main (: n Int64))
              (match (extrema (Map.to-list (fill n Map.empty)) 999 -999)
                ((tuple lo hi) (+ (* 1000 lo) hi))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 3099 Int64)))
