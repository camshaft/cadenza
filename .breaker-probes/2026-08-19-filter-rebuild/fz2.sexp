(case "fz2 the filtered rebuild EQUALS the direct even-only build (selection is canonical)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 2)))))
            (def (fill-even (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill-even (- i 1) (if (= (% i 2) 0) (Map.insert m i (* i 2)) m))))
            (def (keep-even (: ps (List (Tuple Int64 Int64))) (: m (Map Int64 Int64)))
              (match ps
                ((list) m)
                ((list h .. t) (match h ((tuple k v)
                  (keep-even t (if (= (% k 2) 0) (Map.insert m k v) m)))))))
            (def (main (: n Int64))
              (do
                (def filtered (keep-even (Map.to-list (fill n Map.empty)) Map.empty))
                (def direct (fill-even n Map.empty))
                (if (= filtered direct) 1 0)))
            (export main)))
  (call   main (: 60 Int64)) (output (: 1 Int64)))
