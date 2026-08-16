(case "pq1 a 100-entry insort with DESCENDING times (worst-case deep splice each insert) stays ordered"
  (input  (do
        (def (insort (: q (List (Tuple Int64 Int64))) (: e (Tuple Int64 Int64)))
          (match q
            ((list) (List.prepend (list) e))
            ((list h .. t)
              (if (<= (. e 0) (. h 0))
                  (List.prepend q e)
                  (List.prepend (insort t e) h)))))
        (def (fill (: i Int64) (: q (List (Tuple Int64 Int64))))
          (if (= i 0) q (fill (- i 1) (insort q (tuple i (* i 7))))))
        (def (check (: q (List (Tuple Int64 Int64))) (: prev Int64) (: n Int64))
          (match q
            ((list) (tuple prev n))
            ((list h .. t) (if (>= (. h 0) prev) (check t (. h 0) (+ n 1)) (tuple -1 n)))))
        (def (main (: k Int64))
          (match (check (fill k (list)) -999 0)
            ((tuple last cnt) (+ (* 1000 (if (= last k) 1 0)) cnt))))
        (export main)))
  (call   main (: 100 Int64)) (output (: 1100 Int64)))
