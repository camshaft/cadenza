(case "gv1 a guard predicate reads THROUGH two container layers (map-in-record scrutinee)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 3)))))
            (def (main (: k Int64))
              (do
                (def st (record (tbl (fill 40 Map.empty)) (floor 50)))
                (match (Some k)
                  ((guard (Some id) (match (Map.lookup (. st tbl) id) ((Some p) (> p (. st floor))) ((None _u) false))) 1)
                  ((Some _id) 2)
                  ((None _u) -1))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 1 Int64))
  (call   main (: 10 Int64)) (output (: 2 Int64))
  (call   main (: 99 Int64)) (output (: 2 Int64)))
